# Roadmap

Parking lot for work we've decided to keep but not do now. Each item links
the session summary that spawned it (see docs/sessions/). Groomed during the
close-out ritual (global `/close-out` skill; this repo's settings in
`.claude/close-out.md`).

- **The Writing Tools override is a workaround for an OS bug.** The app adds
  `allowsWritingToolsAffordance` → NO to wry's WKWebView subclass because
  macOS 26 asserts inside the affordance it would otherwise float over a
  selection. Two ways this stops being ours: Apple fixes the assertion (re-test
  by deleting the block and selecting text — the Campo lines in the unified log
  are the tell), or wry/tauri exposes
  `WKWebViewConfiguration.writingToolsBehavior`, at which point one config line
  replaces the added method. Until then, leave it: the failure mode is the app
  aborting mid-keystroke.
  _From: [2026-08-29 writing-tools-crash](docs/sessions/2026-08-29-writing-tools-crash/summary.md)_

- **The status-check stream (#171–#174) shipped with its manual passes still
  owed.** Two things the mock cannot express: one `app/scripts/sandbox.sh
  --app` pass on main (click-select feel, the row ▸ hover, ⋯ Close, the
  status sheet, ONE real Ask Claude run against real latency), and
  `docs/ai-profiles-manual-checks.md` **§11** — six checks for the repo's
  first headless claude spawn, of which fail-closed (no spawn, clean error)
  and timeout (no orphan; proves the `exec` in the sh line) are the
  load-bearing pair.
  _From: [2026-08-29 status-check](docs/sessions/2026-08-29-status-check/summary.md)_

- **Watch the nav tiers now that clicks don't spawn sessions.** Auto-open was
  why everything read "active" and pinned/active/dormant felt useless. If
  Active/Idle don't regain meaning after living with #171, the fallback is
  grouping by activity buckets (Pinned / Recent / Stale / Dormant off
  `activityAt`, ignoring tmux) — design sketch in the archived spec. Related
  cheap follow-ups from the same spec, all deferred deliberately: fold the
  health verdict into the MCP `place_status` tool; `worktrees status --ai`
  (headless report from the CLI — the seam exists now); a workspace-wide
  triage sweep ("all my worktrees, one claude -p").
  _From: [2026-08-29 status-check](docs/sessions/2026-08-29-status-check/summary.md)_

- **The recorded media still teaches single-click-opens.** The three
  `app/scripts/record-*.py` / `shoot-profiles.py` scripts were fixed to
  double-click in #172, but the README/feature-page media they produced
  predates click-select — regenerate next time media is touched.
  _From: [2026-08-29 status-check](docs/sessions/2026-08-29-status-check/summary.md)_

- **The mock harness throws an xterm `'dimensions'` error when a mock
  terminal mounts** (`Uncaught TypeError: Cannot read properties of
  undefined (reading 'dimensions')`, from addon-fit). Pre-existing — proven
  identical on base during #172 with the change stashed and the served file
  verified token-free. Harmless to the checks so far, but it pollutes every
  console assertion with a known-noise entry; worth killing next time
  someone is in the harness terminal path.
  _From: [2026-08-29 status-check](docs/sessions/2026-08-29-status-check/summary.md)_

- **The dock-overlap fix has not been seen on WKWebView.** PR #157 is verified
  exhaustively in the mock harness (four resize paths, `getBoundingClientRect`,
  overhang 0) — but the bug was REPORTED from the real app, and every
  measurement behind the fix is Chromium's. Low risk: the flex
  automatic-minimum-size rule and xterm's inline `.xterm-screen` width are
  engine-independent, and both engines agreed on the broken behaviour. Still
  unobserved. One `app/scripts/sandbox.sh --app`, open the dock, look at the
  seam — and the same pass can close the Files → Open item above, which wants
  the same sandbox for a different click.
  _From: [2026-08-18 terminal-dock-overlap](docs/sessions/2026-08-18-terminal-dock-overlap/summary.md)_

- **The column floors are px constants, and page zoom moves the viewport under
  them.** `RAILS_W 88 + NAV_MIN 220 + MAIN_MIN 420` needs 728 CSS px; ⌘+ (PR
  #169) shrinks the CSS-px viewport, so 3× wants a ~2200px physical window and
  `fitLayout` degrades by dropping the dock but never the nav — below that,
  `mainW` goes negative and the topbar piles up. Recoverable (⌘− is ungated, so
  no zoom level can trap you) and unreachable on the display this was built for,
  which is why #169 shipped without it. Same family: `ui_rem` now reaches 22px
  and `term_size` 24px inside the same unchanged floors, with no
  `getComputedStyle` assertion for the new maxima. Wants one decision about what
  the floors mean when the viewport is elastic — probably deriving them from
  `--ui-rem` — not a patch per symptom. Also noted there: a chord pressed during
  the pre-hydration window records into `preHydration` and overwrites the loaded
  value, so ⌘+ at launch can land you BELOW a persisted 2×. One fast IPC wide,
  and shared by every pre-hydration chord (⌘B has it too).
  _From: [2026-08-29 app-zoom](docs/sessions/2026-08-29-app-zoom/summary.md)_

- **The static check scripts run only when someone remembers.**
  `termfit-check.mjs`, `dnd-check.mjs`, `relpath-check.mjs`, `ctxmenu-check.mjs`
  and now `zoom-check.mjs` guard invariants the suites structurally cannot see
  (a CSS declaration, a Rust↔TS mirror, a pure function, a layout effect's dep
  list, a chord's modifier routing) — and none is wired into `ci.yml`, the
  Makefile or `app/package.json`. They are pure Node, no browser, no fixtures,
  sub-second: one CI step running all five costs nothing. Raised in the #157
  review as informational and left out of that PR because it is a repo-wide
  call, not a bug fix's business; #156 added a fourth without changing that, and
  #169 a fifth. Note `race-check.mjs` is NOT in this set — it
  drives real source under controlled promise orders and is slower/noisier.
  _From: [2026-08-18 terminal-dock-overlap](docs/sessions/2026-08-18-terminal-dock-overlap/summary.md),
  [2026-08-18 remove-dialog-menu-clamp](docs/sessions/2026-08-18-remove-dialog-menu-clamp/summary.md),
  [2026-08-29 app-zoom](docs/sessions/2026-08-29-app-zoom/summary.md)_

- **`remove_place`'s `force` is two permissions wearing one flag, and that is a
  core problem now that a UI exposes it.** `ops.rs:1014` reads it as "remove a
  dirty tree"; `ops.rs:1055` reads the same bool to pick `git branch -D` over
  `-d`. So force+`--branch` force-deletes an UNMERGED branch — the only
  combination in the remove path that can destroy commits. The inline arm this
  repo shipped for a year was immune only because it hardcoded `force: false`;
  #156's dialog exposes force, and had to paper over the overload in COPY
  (`forceDeletesBranch` re-words the checkbox) because nothing at the UI layer
  can separate two meanings of one CLI flag. Splitting it (`--force` for the
  tree, `--force-branch` for `-D`) would make the guarantee structural instead
  of editorial. Nothing automated catches a regression here — bats models no
  branch objects and the mock says so out loud.
  _From: [2026-08-18 remove-dialog-menu-clamp](docs/sessions/2026-08-18-remove-dialog-menu-clamp/summary.md)_

- **The remove dialog's reworded warnings have never been seen rendered.** The
  harness pass covered the PRE-review copy; after the rewrite, verification was
  gates + reading only (starting vite was blocked mid-session). Three
  conditionals have never been on screen: the force-on branch label
  ("force-deleted, merged or not"), and the detached-HEAD and force-delete
  variants of the commits note. One `pnpm dev:mock` with a detached fixture and
  both boxes ticked closes it — and the same pass can check the dialog against
  #155's `.nw-*` sibling, which landed in the same shell mid-flight.
  _From: [2026-08-18 remove-dialog-menu-clamp](docs/sessions/2026-08-18-remove-dialog-menu-clamp/summary.md)_

- **Nothing checks the new-worktree verdict against `ops.rs`.** The dialog's
  verdict line reimplements `cmd_new`'s decision ORDER by hand — and that order
  is not the obvious one (core reaches its holder logic only when the derived
  directory does not exist, `ops.rs:417`), so the first version got four cases
  wrong before review caught them. This is the same exposure `dnd-check.mjs`
  exists to cover for `store::reconcile`, minus the checker: a change to core's
  create path leaves a frontend that is confidently wrong and passes every test.
  A parser-based check over `cmd_new`'s branches is the obvious fix if the chain
  grows again.
  _From: [2026-08-18 new-worktree-dialog](docs/sessions/2026-08-18-new-worktree-dialog/summary.md)_

- **`remote_only` on `BranchList`, so the verdict can say "tracking".**
  `branch_names()` unions local heads with origin-only branches and strips the
  prefix, so the frontend cannot distinguish "this branch exists locally" from
  "this will be tracked from `origin/`" — two different acts that currently read
  identically ("already exists — it will be checked out here"). One extra field
  on the struct; the switcher's create row would get the same benefit.
  _From: [2026-08-18 new-worktree-dialog](docs/sessions/2026-08-18-new-worktree-dialog/summary.md)_

- **One real-app pass over the new-worktree dialog.** Merged without it: the
  sandbox app is a native WKWebView window and Playwright speaks CDP, so the
  interactive pass needs a person. Every state was asserted in the mock harness
  and the one real-timing risk (a slow `list_branches` making the verdict guess)
  was found and fixed by simulating the delay — but the harness cannot express
  what the real git fan-out does. `app/scripts/sandbox.sh --app`, hit `+`, and
  try: a new branch, an existing one, a branch another worktree holds, and
  `main`.
  _From: [2026-08-18 new-worktree-dialog](docs/sessions/2026-08-18-new-worktree-dialog/summary.md)_

- **A layout change while a shell tab is detached still replays at the wrong
  width.** PR #153 stops the pane attaching at a size that isn't its own, which
  was the reproducible case — but a ring written at one width and replayed
  after a REAL change (⌘B while flipped away, a window resize) is still raw
  bytes laid out for a grid that no longer exists, and no byte log replays
  faithfully across that. Ages out via the 256K cap. The full fix is
  terminal-state serialization (a server-side screen model, replay state not
  bytes) — parked because the degraded case is now rare and self-healing.
  _From: [2026-08-17 gitignore-cmdt-replay](docs/sessions/2026-08-17-gitignore-cmdt-replay/summary.md)_

- **Existing stores never get the `.git/info/exclude` entries.** PR #150 writes
  them only when `store::edit` CREATES the places file, so every repo that
  already had one keeps answering `git status` with an untracked
  `.worktrees.places.json` until hand-ignored. Deliberate scope cut, not an
  oversight — the candidate fix is a one-time sweep (app startup, or project
  add) running the same best-effort append for every known project root; it is
  idempotent and cannot hide a tracked file, so the only real question is where
  to hang it.
  _From: PR [#150](https://github.com/penard-monkey/worktrees/pull/150)_

- **One real-app click on Files → Open for a dot-component path.** The mock
  stubs `plugin:opener|*`, so PR #151's open-path grant (scoped permission +
  `requireLiteralLeadingDot: false`) is proven by plugin-source reading and a
  standalone glob probe, never end-to-end. `app/scripts/sandbox.sh --app`,
  right-click a file under `.worktrees/…`, hit Open — one minute, and it
  closes the only unverified edge of that PR. (The sandbox pass that followed
  exercised the menu's copies, not Open.)
  _From: PR [#151](https://github.com/penard-monkey/worktrees/pull/151)_

- **Sync v2: SSH targets, push-all, scheduling.** The hub abstraction was built
  not to preclude `rsync -e ssh` (direct Mac→Mac without carrying the SSD); a
  workspace-level "push every project" and a scheduled/auto sync are the other
  two deliberately-parked asks. All three ride the existing engine — the work
  is target plumbing and UX, not transfer logic.
  _From: [2026-08-16 sync-macs session](docs/sessions/2026-08-16-sync-macs/summary.md)_
- **The heal matcher skips patterns rsync would anchor.** A user
  `extra_excludes` entry containing `/` (e.g. `docs/*.log`) is honored by rsync
  but deliberately unmatched by the post-pull heal (conservative-by-choice), so
  a tracked file it excluded stays "deleted" after an import. Fine for the
  builtin set (no such shapes); becomes real the day someone writes one. Fix =
  teach `excluded_path` transfer-root anchoring.
  _From: [2026-08-16 sync-macs session](docs/sessions/2026-08-16-sync-macs/summary.md)_
- **Two small sync leftovers.** `~/bin/sync-macs` still exists — David deletes
  it manually (its retirement gate passed at phase 1). And the import picker
  cannot reach the "destination exists locally but isn't a workspace project"
  case; core's guards cover it, but it has never been walked by hand.
  _From: [2026-08-16 sync-macs session](docs/sessions/2026-08-16-sync-macs/summary.md)_
- **Glance at ⌘F highlights on emoji-bearing lines after the next release.**
  The emoji-width fix (graphemes addon) and the search addon landed in
  separate sessions and were only eyeballed apart — the sandbox pass for the
  width fix predated its rebase onto search. They are logically independent
  (graphemes loads before `term.open`, search after), so this is a
  one-minute confirmation, not an investigation: ⌘F for a word on a line
  with ✅/⚠️ in tmux, decorations must sit on the match.
  _From: [2026-08-17 emoji-width](docs/sessions/2026-08-17-emoji-width/summary.md)_

- **Smoke nav drag & drop in the real app.** Everything about it was verified in
  the mock harness with synthetic pointer events, and the mock answers in a
  microtask while the real `list_workspace` is a git fan-out of seconds — the
  exact blind spot CLAUDE.md's three v0.12.x bugs came from. No tooling in the
  session could drive a pointer in a native WKWebView window, so this is a hand
  pass: `app/scripts/sandbox.sh --app`, then in order — (1) drag a place from
  Active onto Pinned, it must stay pinned through the refresh that follows;
  (2) drag a pinned place onto Idle, it should unpin, land in Active and say
  why; (3) drop onto Active, nothing moves and the hint appears; (4) reorder two
  projects and see the order survive the confirming `list_workspace`; (5) drag a
  row to the nav's top/bottom edge for auto-scroll; (6) hover a collapsed group
  mid-drag for spring-open; (7) undo once after a tier change. Items 1, 2 and 4
  are the ones the harness structurally cannot express.
  _From: [2026-08-14 drag-drop-nav](docs/sessions/2026-08-14-drag-drop-nav/summary.md)_

- **The drag ghost re-renders all of App, once per frame.** `setDrag` on every
  rAF re-renders a 3500-line component for the duration of a drag. Measured
  acceptable on this machine and deliberately not optimised; a portal for the
  ghost (and gap state kept out of App) is the fix if it ever bites on a bigger
  workspace or a slower machine.
  _From: [2026-08-14 drag-drop-nav](docs/sessions/2026-08-14-drag-drop-nav/summary.md)_

- **A place dragged onto the nav's own padding does nothing, silently.** Project
  drags now accept anywhere inside `.nav-scroll`, but a PLACE row dropped
  between two groups (or below the last one) resolves no tier, so there is no
  gap and no drop. That is honest — no group means no target — but the empty
  space below a project's last group is a plausible place to aim for "the end of
  that group". Worth deciding whether the nearest group should claim it.
  _From: [2026-08-14 drag-drop-nav](docs/sessions/2026-08-14-drag-drop-nav/summary.md)_

- **Find is deliberately narrow in three places.** ⌘F on an image or binary
  viewer opens a bar that can only ever answer "no results" — it should be
  suppressed by file kind. Terminal find sees only what xterm received since it
  attached (default `scrollback` 1000 lines); raising that widens it, while
  reaching tmux's own history would mean driving copy-mode, which takes over the
  user's pane and was deliberately not attempted. And the file side's
  no-Highlight-API fallback — select the active hit instead of tinting all of
  them — exists for WKWebView older than Safari 17.2 and has never been
  exercised; there is no `minimumSystemVersion` in `tauri.conf.json` to say
  whether anyone can reach it.
  _From: [2026-08-14 cmd-f-find](docs/sessions/2026-08-14-cmd-f-find/summary.md)_

- **No app chord is guarded against the surfaces above it except the palette and
  Settings.** The keydown handler checks `switchOpen`/`settingsOpen`, so ⌘J, ⌘B,
  ⌘1-9, ⌘E and now ⌘+/⌘−/⌘0 all still fire behind the What's-new scrim and the
  ProjectSheet — rearranging panels nobody can see. Long-standing and uniform,
  which is why it keeps not being worth a one-chord fix; the fix is a single
  "is a modal on top" predicate for all of them. Two smaller siblings from the
  same session: a disabled stepper button never shows its `title`, so the
  ⌘−/⌘+ hints vanish exactly at the ends of the range where a user is most
  likely to hunt for them.
  _From: [2026-08-14 markdown-zoom](docs/sessions/2026-08-14-markdown-zoom/summary.md)_

- **The markdown zoom chord has never been pressed in a real build.** Every
  ⌘+/⌘−/⌘0 assertion came from synthetic `KeyboardEvent`s in Chromium under
  Playwright; the buttons are unaffected either way. Static evidence says the
  chord reaches JS (no native menu, no `zoomHotkeysEnabled` in `tauri.conf.json`),
  and `app/scripts/sandbox.sh --app` closes it in about 30 seconds. Worth folding
  into the next real-app run rather than making its own trip. The general gap is
  the known one: the mock harness cannot answer any question about what WKWebView
  does with a key before the page sees it. **Partly answered since:** the ⌘F
  session confirmed a ⌘-chord does reach the page in a real build (⌘F is the only
  way to open the find bar, and it opened), so the remaining doubt is specific to
  ⌘+/⌘−/⌘0, which browsers normally claim for zoom.
  _From: [2026-08-14 markdown-zoom](docs/sessions/2026-08-14-markdown-zoom/summary.md)_

- **A tab index is an identity split across two files, and only one of them is
  swept.** A dock tab's NAME and STRIP live in `ui-state.json` (frontend), its
  DIRECTORY in `shell-cwds.json` (backend), and the two halves are dropped by
  different code — `closeTab` drops all three, `remove_place` drops both sides,
  but every other path drops one and keeps the other. That asymmetry produced
  two of the six defects this session (a new tab inheriting a remembered
  index's name and directory; untracking a project destroying the frontend half
  while the backend kept its own). The remaining accepted case is a place
  removed and recreated while the app is closed. Worth considering whether tab
  identity should live in ONE place — most likely the declared store — rather
  than being kept in sync by convention.
  _From: [2026-08-14 terminal-tab-memory](docs/sessions/2026-08-14-terminal-tab-memory/summary.md)_

- **The mock harness models no shells, no directories and no time.** It CAN
  model `close_place`/`open_place`, and the two session-down defects this
  session got through only because nothing had ever driven it there — a gap in
  what we exercise, not in what the harness can express. A short scripted pass
  over the state machine's transitions (session up/down, place switch, add,
  close, restart) would have caught both, and would keep catching them; the
  Playwright runs here were hand-driven each time.
  _From: [2026-08-14 terminal-tab-memory](docs/sessions/2026-08-14-terminal-tab-memory/summary.md)_

- **A just-created worktree wears the base tip's age** — "5d" seconds after
  creation, until work or a commit lands in it. Accepted, but the blast radius
  grew: this used to be a nav-tree quirk, and now that ⌘K, the Recent lens and
  the home Resume list all rank on `activityAt`, a fresh place sits wherever its
  base branch's last commit puts it in **every** list. The fix, if it ever reads
  wrong in practice, is a creation epoch in the declared store — not a special
  case in the clock. (The other half of this item, "⌘K and Resume still rank on
  opens", was closed by the cmdk-activity-order session.)
  _From: [2026-08-10 nav-activity-age session](docs/sessions/2026-08-10-nav-activity-age/summary.md),
  widened [2026-08-11 cmdk-activity-order](docs/sessions/2026-08-11-cmdk-activity-order/summary.md)_

- **⌘K lists `(main)` places; the Recent lens filters them out.** Both survived
  the ordering unification untouched, so the asymmetry is now the only thing
  left that makes those two lists disagree. Arguably right — the switcher is for
  jumping anywhere, Recent is for resurfacing work — but it has never actually
  been decided. Worth one deliberate call rather than leaving it as an artifact
  of two different filters.
  _From: [2026-08-11 cmdk-activity-order](docs/sessions/2026-08-11-cmdk-activity-order/summary.md)_

- **The backend 3 s poll loop is the largest remaining background cost.** v0.9.1
  gated every FRONTEND periodic cost on window visibility, which is what killed
  the git-sweep storm — but the setup thread in `app/src-tauri/src/lib.rs` still
  runs `session_fingerprint()` (a `tmux list-sessions` spawn) plus
  `claude_activity()` (read_dir + parse + `pid_alive` per probe) **every 3
  seconds regardless of visibility**, and the auto-fetch pass fires a `git fetch`
  per root on the same ungated thread — real battery on a docked laptop
  overnight. The shape to copy already exists: push a mode from the frontend the
  way `set_fetch_interval` does, 3 s focused → 20-30 s unfocused → parked when
  hidden, with one immediate tick on the visible edge. Measured floor: a hidden
  v0.9.1 window still spawned tmux on this cadence.
  ⚠ **Since v0.10.0 this loop also carries the afterglow's completion edge**
  (`completion_edges` in lib.rs), which is exactly what lets a task finishing
  while the window is hidden still light its dot. Parking the loop when hidden
  would silently drop those completions until the next launch's backfill —
  slowing the cadence is fine, stopping it is not, and `claude_activity()` is
  the part that has to keep running.
  _From: [2026-08-07 power-consumption session](docs/sessions/2026-08-07-power-consumption/summary.md), amended [2026-08-09 afterglow-dot](docs/sessions/2026-08-09-afterglow-dot/summary.md)_

- **The afterglow's signal path has no automated coverage, by necessity.** There
  is no fake `claude`, so the busy→ember hand-off and the `history.jsonl`
  backfill are proven only by construction (unit tests on the pure pieces) and
  against the mock harness. `docs/ai-profiles-manual-checks.md` §10 is the
  vehicle — nine checks, **not yet run against the real app even once**. The
  load-bearing ones: a session opened but never prompted must stay dark, `claude`
  run from a subdirectory must light the PLACE and not invent a store entry named
  after the subdir, and `/clear` must neither light a place nor inflate an
  existing ember. Re-run whenever the `claude` binary is upgraded — the probe
  schema and the history format are both undocumented.
  _From: [2026-08-09 afterglow-dot session](docs/sessions/2026-08-09-afterglow-dot/summary.md)_

- **Zombie children — real, unreproduced, and the obvious diagnosis is wrong.**
  A 14-hour v0.9.0 instance accumulated 41 unreaped children. `term_close` /
  `kill_shell` call `child.kill()` with no `wait()`, which looks like the bug and
  is not: `portable_pty`'s `ChildKiller::kill` sends **SIGHUP** then polls
  `try_wait()` five times over ~200 ms, and `try_wait` reaps. The genuine hole is
  the fall-through — a child outliving that grace period gets `SIGKILL` and no
  final wait. Four reproduction attempts failed (plain `sleep` via PTY,
  `trap "" HUP`, enumerating all children, standalone outside the test harness),
  so a written fix was reverted rather than shipped under a claim it hadn't
  earned. ⚠ The regression test written for it passed identically with and
  without the fix — verify any guard FAILS on the unfixed code first. Live check
  in the session summary; grab a reproduction when the count climbs.
  _From: [2026-08-07 power-consumption session](docs/sessions/2026-08-07-power-consumption/summary.md)_

- **The nav is rebuilt, not re-rendered.** `PlaceRow`, `GroupHeader`,
  `ProjectNode` and `FlatLens` are defined inside `App()` (`app/src/App.tsx`), so
  every App render gives them new identities and React unmounts and recreates the
  entire subtree — ~450-600 DOM nodes, on every refresh, every busy-dot flip and
  **every keystroke in the filter box**. The comment at the definition site
  documents the exception and justifies it on correctness grounds (no local
  state, no focus), which is sound and silent about DOM churn. All four must move
  to module scope together with `memo` — hoisting the leaf alone achieves
  nothing, since a redefined `ProjectNode` unmounts its children regardless.
  Held back from v0.9.1 deliberately: ~30 closed-over values, invisible to bats,
  and the energy measurements never implicated render churn — so measure before
  spending the refactor.
  _From: [2026-08-07 power-consumption session](docs/sessions/2026-08-07-power-consumption/summary.md)_

- **Event-driven place refresh (FSEvents) instead of the 30 s blind emit.** The
  backend force-emits `places:changed` every 30 s as a safety net for drift with
  no tmux trace — an editor writing files, a commit from a bare terminal. All of
  those are filesystem events under the project root, and worktrees nest under
  it, so one recursive `notify` watch per project covers the working trees and
  the `.git` common dir. Would cost ~zero when nothing changes and make the UI
  update in under a second instead of up to 30 — strictly better on both axes.
  Adds a dependency; keep a slow backstop for FSEvents edge cases. Same watcher
  would replace the claude probe-dir scan.
  _From: [2026-08-07 power-consumption session](docs/sessions/2026-08-07-power-consumption/summary.md)_

- **PTY → IPC coalescing (terminal throughput).** Both reader threads send one
  `Channel::send` per `read()` syscall. In tauri 2.11.5
  (`src/ipc/channel.rs:155-183`) every send becomes a webview main-thread JS
  eval, and raw payloads **under 1024 B are serialized by `serde_json` into a
  JSON array of integers** (~4 chars per byte) — so a PTY under a build emits
  few-hundred-byte reads at kHz rates and pins the WebContent process. Coalesce
  with a state machine that cannot hurt echo latency: send immediately when idle,
  and only batch (8-16 ms or 32-64 KB) once reads arrive back-to-back. Bursts
  then clear 1024 B and take the cheaper-per-byte path, and xterm gets fewer,
  larger writes. Optional companion: `@xterm/addon-webgl` with
  `onContextLoss → dispose` (DOM renderer is the sanctioned fallback) — invisible
  to bats, so it needs a manual-check doc entry.
  _From: [2026-08-07 power-consumption session](docs/sessions/2026-08-07-power-consumption/summary.md)_

- **Flaky test: `skills add installs from a local git repo and pins the commit`**
  (`test/skills.bats:175`) failed on macOS CI only, passed locally and on Linux,
  and passed clean on re-run. A `git clone file://` test. Worth pinning down
  before it trains everyone to re-run CI on red.
  _From: [2026-08-07 power-consumption session](docs/sessions/2026-08-07-power-consumption/summary.md)_

- **Links in release notes still print raw.** `renderInline` (App.tsx) now
  handles `code`, `**strong**` and `*em*` — the markup the changelog bullets
  actually use — but not `[text](url)`. Nothing renders wrong today: links
  appear in the changelog's file header, never inside a `### ` group's bullets,
  which is all the sheet renders. The first entry that links a PR or an ADR will
  show the brackets. Not free to add: a rendered href needs `safeHref`-style
  vetting and click routing, because a click handler alone is not a boundary
  (middle-click fires `auxclick` with no `click`, so `preventDefault()` never
  runs and the WebView follows the attribute) — `markdown.tsx` already solved
  this and is the place to copy from.
  _From: [2026-08-07 release-notes markdown session](docs/sessions/2026-08-07-relnotes-inline-markdown/summary.md)_

- **`do_switch` still pays the doomed two-fetch pair.** `cmd_new` was fixed to
  ask the remote once (`ops.rs`, guarded single `git fetch origin`), but
  `do_switch` (`ops.rs:309-321`) keeps the old shape: a targeted
  `fetch refs/heads/<branch>:refs/remotes/origin/<branch>` — the request that
  cannot succeed for a branch being invented — followed by a separate base
  fetch. It matters beyond `switch`, because `new` ROUTES through it when the
  worktree already exists on another branch, so that path still costs two
  network waits. Same three-line fix, but it changes `switch`'s own semantics
  and goldens (`test/switch.bats`), so it was left out of the `new` change
  rather than smuggled in. Note the fork it leaves: a `--single-branch` clone no
  longer force-materializes a remote branch via `new`, but still does via
  `switch`. Related: on that same holder-reuse path the nav's new pending row is
  suppressed by the slug dedup (the place already exists), so the user gets
  neither the spinner nor the speedup — marking the EXISTING row busy would be
  the fix, not a second ghost.
  _From: 2026-08-07 ui-next session (new-worktree delay + warn spam)_

- **App signing + notarization** — bundles ship unsigned; install.sh strips
  quarantine on a checksum-verified install. The proper distribution tier is
  Developer ID signing + notarization so Gatekeeper passes without the
  workaround. (release.yml calls this "the later distribution tier".)
  Shape: an Apple Developer membership ($99/yr) → a **Developer ID Application**
  cert exported as `.p12` → `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
  `APPLE_SIGNING_IDENTITY` plus notarization credentials (`APPLE_ID` + an
  app-specific password + `APPLE_TEAM_ID`, or an App Store Connect API key) as
  repo secrets; tauri-action picks these up and runs `notarytool` itself. Note
  `TAURI_SIGNING_PRIVATE_KEY` is NOT this — it is the minisign key for updater
  artifacts, and the workflow's "signed" log lines refer to that.

  Second reason to do it, found 2026-08-13: it is also the fix for macOS
  **privacy** prompts. TCC keys an approval to the designated requirement, which
  for an ad-hoc signature is the cdhash, so every upgrade voids every grant a
  user has given. A Developer ID signature makes the requirement cert-based and
  the grants durable — for everyone, not just whoever signs their own build.

  **The Mac App Store is not an alternative route** (evaluated 2026-08-13,
  rejected): MAS requires App Sandbox, and a sandboxed process may exec only
  binaries inside its own bundle, with children inheriting the sandbox. That
  forbids shelling out to `git`/`tmux` — the stated architecture — and
  `fixup_gui_path()` exists precisely to reach homebrew's tmux, which a container
  cannot read. Worse, the tmux server would live in the container: the CLI and
  the user's own shell could no longer attach to the app's sessions, which is the
  product. Also incompatible: spawning the user's `claude` binary, reaching
  `~/.gitconfig` and `~/.ssh` for fetch/push, the self-updater (prohibited), and
  the Settings → Version button that installs a CLI onto `PATH`. Sandboxing would
  "fix" the privacy prompt by making the access impossible.
  _From: [2026-07-26 themes session](docs/sessions/2026-07-26-themes-v0.2.4/summary.md),
  expanded [2026-08-13 codesign session](docs/sessions/2026-08-13-codesign-privacy-prompts/summary.md)_

- **A parked branch signs LOCAL installs** — `bug-fixes-codesign-local-installs`
  (pushed; [PR #124](https://github.com/penard-monkey/worktrees/pull/124), closed
  not merged). Adds `SIGN_ID` to the Makefile and `WORKTREES_SIGN_ID` to
  install.sh: both re-sign what gets installed with a cert-backed identity, so
  TCC approvals survive rebuilds. Gated and verified in three states each (see
  the session summary). Parked because it fixes one machine and the same problem
  is solved for everyone by the entry above — but the Makefile half stays useful
  even after Developer ID lands, because a CI-signed release says nothing about
  what `make install-app` puts in /Applications. Resurrect with `gh pr reopen 124`,
  or fold it into the Developer ID work.
  _From: [2026-08-13 codesign session](docs/sessions/2026-08-13-codesign-privacy-prompts/summary.md)_

- **install.sh could mint a per-machine signing cert** — the zero-cost version of
  the above, for users with no Apple membership: generate a self-signed Code
  Signing cert once (`openssl req -x509` → `security import` into the login
  keychain with `-T /usr/bin/codesign` → `security set-key-partition-list` so
  codesign can use it without a GUI prompt), then sign every install with it.
  TCC only checks that the requirement MATCHES, not that the cert is trusted, so
  this makes privacy grants survive upgrades without Gatekeeper being involved at
  all. Cost: it needs the user's keychain password once, which is a lot of
  ceremony for an installer — worth doing only if Developer ID stalls.
  _From: [2026-08-13 codesign session](docs/sessions/2026-08-13-codesign-privacy-prompts/summary.md)_

- **Stale worktree/branch cleanup** — `New-icon` and `spike-demo` worktrees
  (and their branches) predate the current stream; decide merge/abandon and
  remove. `release-v0.2.3` worktree is superseded by v0.2.4. Plus a pile of
  merged/abandoned REMOTE branches from earlier streams still on origin
  (`feat/app-install`, `feat/app-self-update`, `feat/in-app-update`,
  `feat/nav-prefs`, `feat/ui-polish`, `feat/ui-redesign`, `fix/gui-path`,
  `fix/term-resize-artifacts`, `fix/ui-responsiveness`, `next`,
  `release/0.2.0|0.2.1|0.2.2`, `docs/*`, `chore/*`, `settings-next`) — squash
  merges break `git branch --merged`, so verify each against its PR before
  `git push origin --delete`. (This session's 9 branches were cleaned at close-out;
  `next-stream` and `release/0.3.2` were cleaned at the 2026-07-28 close-out.)
  _From: [2026-07-26 themes session](docs/sessions/2026-07-26-themes-v0.2.4/summary.md)_

- **cdv migration — the live hazard is still live.** Per-project settings shipped
  (#58), but `casa-del-valle-monorepo` has not been migrated, so it still runs
  two worktree tools. Two of its worktrees (`claude-work-integration`,
  `prod-reviews`) have no `.worktree.env`, and that repo's `deploy-local.sh`
  reads that absence as "not a worktree" → a global `pkill -9` that kills the
  main checkout's whole running stack. Eleven more lack `WEBSITE_PORT` and bind
  main's 3002.
  Runbook (8 steps, rollback each) and the transcribed config are archived at
  `docs/sessions/2026-07-28-project-settings/{RUNBOOK.md,cdv.worktrees.toml}`.
  **Blocked on step 0:** verify `apps/mobile/google-services.json` (sender id
  `86759926600`) against the Firebase console — the old script's `relink --all`
  replaced `general-fixes`' only real copy with a symlink on 2026-07-27 21:33
  and `ln -sfn` leaves no backup.
  ⚠ **Do not use `worktrees init` there.** Verified against the real repo: it
  suggests `apps/backoffice/.env.local` as a link (must be `copy` — a script
  rewrites it, so a link writes through to main and breaks every worktree at
  once) and emits `POSTGRES`/`LOCALSTACK` where the scripts read
  `PG_PORT`/`LS_PORT`, missing `WEBSITE` and `META_MOCK`. init now warns about
  both classes but cannot infer them.
  Also out of reach: 4 registered worktrees outside `.worktrees/` (three under
  `.dmux/`, one sibling) that `provision --all` cannot see.
  _From: [2026-07-28 project-settings session](docs/sessions/2026-07-28-project-settings/summary.md)_

- **Project-settings polish** — small, all from the same session:
  - `app/src/mock/install.ts`'s `SUGGESTED_TOML` is a hand-written fixture, not
    a mirror of `init::render()`, so the harness preview lacks init's warnings.
    No user-facing drift (production uses the real emitter).
  - A refresh-raised error is retracted by identical-string match; two sources
    producing byte-identical text would cross-clear.
  - The 4s arm auto-disarm is tight for "Kill &lt;session&gt; — whole session?".
  - `doctor`'s session-drift scan skips `(main)` on named-place runs.
  - Spec §4 still says "the same rules apply to `[compose] file`" — true per
    entry, but the key is `files` now.
  - `store.rs`'s `DirLock` still uses 15s-mtime staleness, contradicting
    `DESIGN.md:205`'s PID-liveness mandate. `provision.rs` deliberately did not
    copy it; the store's own wart remains.
  _From: [2026-07-28 project-settings session](docs/sessions/2026-07-28-project-settings/summary.md)_

- **Global summon hotkey (v0.3.2)** — OS-level chord (tauri global-shortcut
  plugin) that fronts the window and drops straight into the ⌘K switcher —
  summon → fuzzy-jump → attach in one motion. Prerequisite (the switcher) now
  exists (#52). Needs capabilities entry + a key-capture setting widget.
  _From: [2026-07-27 settings session](docs/sessions/2026-07-27-settings-and-audits/summary.md)_

- **Work-stream framing sweep** — README + repo description + app tagline now
  say "a durable place for every work stream"; DESIGN.md, install.sh output,
  and remaining app copy still lead with "one worktree per branch". Align.
  _From: [2026-07-27 busy-dots session](docs/sessions/2026-07-27-busy-dots-nav-home/summary.md)_

- **Eyeball busy dot in the real app** — the dot was rewritten this session
  (#49): now reads `~/.claude/sessions/*.json` (busy→green blink,
  waiting→amber). Validated via mock harness + screenshots; still not observed
  in the real Tauri app against a live claude session (probe-file read + PID
  liveness are the untested-in-prod paths). **Now carries a second question**:
  the park-residue guard (#140) has also never run against a real probe. Both
  answer in one pass — write two probe files against live pids, one `busy` with
  `parkedJobId` and skewed stamps (must stay dark) and one `busy` with equal
  stamps (must light), then `app/scripts/sandbox.sh --app`. The natural sample
  was deleted to unstick the dot before anyone thought to keep it.
  _From: [2026-07-27 settings session](docs/sessions/2026-07-27-settings-and-audits/summary.md),
  widened [2026-08-17 parked-busy-dot](docs/sessions/2026-08-17-parked-busy-dot/summary.md)_

- **Does a parked `waiting` probe pin the amber dot too?** The park-residue fix
  (#140) guards the `busy` arm of `claude_activity` only. If a session can be
  parked while its status is `waiting`, the same stale-status residue would pin
  an amber "needs input" dot forever — one word to fix (`"waiting" if
  !delegated`), but no probe on this machine has ever shown that shape and it is
  unknown whether the CLI even allows parking from a blocked prompt. Find out
  before adding the guard; a wrong guess darkens a dot that should be lit.

- **Nav / settings backlog (from 2026-07-27 audits)** — verified but not-yet-done
  items from the settings evaluation + ctx-menu audit. Full detail in that
  session's archived `findings.md` (`planning.tar.gz`). In rough priority:
  - _Confirmed bugs:_ `note-focus-terminal-steal` (note keystrokes land in the
    shell when editing a note on a non-selected live place); `github-url` branch
    not percent-encoded (`#`/`%` → wrong page).
  - _UX gaps:_ success/warning output discarded by `runCmd` (branch-steal on name
    collision, remove scope lines); dirty-remove says "pass --force" the GUI can't;
    no EOF signal → out-of-band detach freezes the terminal; "Close" is the secret
    lifecycle reset (no affordance says so); "Open on GitHub" mislabeled + drops
    branch link for GitLab/Bitbucket; open ctx-menu reshuffles on the 3s poll;
    native WKWebView menu leaks over popovers.
  - _Zero-knob fix:_ `origin/HEAD` base detection to replace the rejected
    default-base setting (+ fix the "base (default: main)" placeholder lie).
  - ~~_Test infra:_ mock fault-injection (no mock `CmdResult` returns `ok:false`, so
    the error-banner path is untestable headlessly).~~ **Done 2026-08-07** — a
    `new_place` branch containing `fail` returns `ok:false`, which is what made the
    rejected-create path testable. Only `new_place` has it; the other commands still
    always succeed, so widen it where a failure path needs covering.
  - _ai-command phase 2:_ editable AI command — needs a comment-preserving
    `cfg_set` writer in worktrees-core first.
  - ~26 polish items (menu keyboard/ARIA, mock parity drifts, minor labels).
  _From: [2026-07-27 settings session](docs/sessions/2026-07-27-settings-and-audits/summary.md)_

- **Better desktop-app README gif** — current `docs/media/desktop-flow.gif`
  is driven against the mock harness, so the embedded terminal shows the
  "mock terminal — design harness" banner. Re-cut from the real Tauri app
  (live tmux + AI CLI in the pane); consider trimming the flow and resting on
  the overview rather than the terminal. Recorder: `app/scripts/record-readme.{sh,py}`.
  _From: 2026-07-27 readme-media session_

- **Dock file viewer: in-app editing, if it is ever wanted again.** The viewer
  is now read-only and renders markdown/code/images. Bringing editing back means
  CodeMirror or equivalent — a real decision, not a default. The read-mode
  highlighter is hand-rolled (`app/src/highlight.ts`) and deliberately
  approximate: it does not parse, so exotic constructs can mis-colour. Known
  gaps: Rust char literals are uncoloured (the same rule that keeps `&'a str`
  lifetimes correct), and `.m` is assumed C-like rather than MATLAB.
  _From: [2026-08-03 files-viewer session](docs/sessions/2026-08-03-files-viewer/summary.md)_

- **Smoke the v0.8.0 Files tab in the REAL app** — shipped on mock-harness
  evidence only. Three things the mock cannot prove: `read_file_base64` against
  real bytes (a multi-MB PNG, the 4 MiB truncation branch), the refusal of
  `file:`/`data:` links in a real WKWebView, and whether WKWebView's middle-click
  follows an href the way Chromium does. Also worth eyeballing a big real README
  and a deep tree for scroll/perf.
  _From: [2026-08-03 files-viewer session](docs/sessions/2026-08-03-files-viewer/summary.md)_

- **A global React error boundary.** There is none above `App` (`main.tsx`
  renders `<App/>` bare), so any uncaught throw unmounts the root and leaves a
  blank window with no way back but a restart. The Files tab now has a local
  `ViewErrorBoundary`, which only covers the viewer body. A top-level boundary
  that shows the error and offers a reload — routed through `applog` — would
  turn a class of crashes into a recoverable pane.
  _From: [2026-08-03 files-viewer session](docs/sessions/2026-08-03-files-viewer/summary.md)_

- **Markdown viewer gaps** (all small, none blocking): only 7 HTML entities are
  decoded (`&copy;`, `&hellip;`, numeric refs render literally — zero occurrences
  in this repo's docs today); heading slugs are GitHub-style but not
  de-duplicated, so two headings with the same text share an id; a 1000-level
  nested list takes ~1.2s to build; footnotes (`[^1]`) render as literal text
  since marked core has no footnote extension.
  _From: [2026-08-03 files-viewer session](docs/sessions/2026-08-03-files-viewer/summary.md)_

- **Smoke the v0.5.0 dock + drag in the real app** — neither is verifiable in
  the mock harness: (1) manual sort drag (fix was `dragDropEnabled:false`;
  WKWebView-only behavior), (2) owned dock shells — replay after tab
  flip / ⌘J, detach-not-kill mid-command, exit → Restart, no orphan `zsh -l`
  after app quit (`ps` check). The tmux-sidecar lifecycle-edges item from the
  2026-07-28 session is superseded: sidecars were replaced wholesale by owned
  PTYs in v0.5.0.
  _From: [2026-07-29 right-panel session](docs/sessions/2026-07-29-right-panel/summary.md)_

- **Right-panel review nits** (deliberately left): `--nav-w`/`--dock-w` CSS vars
  still written by App's layout effect — ⚠ **"nothing consumes them" is wrong**,
  and acting on it as written would have shipped a bug: `App.css:30` uses
  `var(--nav-w)` in the base `.app` rule and `tokens.css:113` defines it as
  `300px`. The inline `gridTemplateColumns` always overrides it, so the base
  rule is a fallback that never applies in practice — but deleting the effect
  without also re-pointing those two lines leaves the fallback resolving to a
  stale 300px. `--dock-w` **is** genuinely dead now (the 2026-08-11 session made
  the dock a flex sibling rather than a grid column). Three files, so it wants
  its own change;
  `place_session_cwd` doc comment still explains sidecar-name stability;
  combobox Enter with a highlight index stale after a filter shrink falls back
  to raw text (acceptable, but a bounds-clamp on `hi` is one line).
  _From: [2026-07-29 right-panel session](docs/sessions/2026-07-29-right-panel/summary.md),
  corrected [2026-08-11 space-workbench](docs/sessions/2026-08-11-space-workbench/summary.md)_

- **tmux gate: manual `[Y/n]` run + PATH hygiene.** The installer's
  interactive macOS brew prompt (install.sh `require_tmux`) has never been
  exercised on a real tty — one live run on a tmux-less mac. Separately,
  every banner Re-check re-prepends the std-dirs block to PATH
  (`fixup_gui_path` is re-entrant now; dups harmless but unbounded) — add a
  dedupe pass, and while there reconsider runtime `std::env::set_var` vs
  handing a computed PATH to spawns (POSIX getenv race, accepted for now).
  _From: [2026-07-30 tmux-gate session](docs/sessions/2026-07-30-tmux-gate/summary.md)_

- **Verify single-pane "New" on a real mac.** PR #70 makes app-created
  places single-pane (no auto-install; `then:` hint instead) — confirmed via
  bats shims only; one real app "New" on a lockfile repo to confirm pane
  layout + hint.
  _From: [2026-08-01 single-pane-new session](docs/sessions/2026-08-01-single-pane-new/summary.md)_

- **Smoke-test the usage widget on a real launch.** v0.7.0's nav-footer
  bars are verified against the endpoint via curl and in the mock harness,
  but the built app's Keychain path (one-time prompt for "Claude
  Code-credentials") hasn't been exercised end-to-end. Now also covers the
  reset countdowns: `resets_at` has only ever been read through the tooltip on
  the real oauth path, so the live `2d 5h` has never been seen outside the mock.
  _From: [2026-08-02 usage-widget session](docs/sessions/2026-08-02-usage-widget/summary.md),
  [2026-08-06 usage-countdown session](docs/sessions/2026-08-06-usage-countdown/summary.md)_

- **CI has never run against several merged changes.** PR #82 merged while
  GitHub's queue was backed up and no workflow run was ever created, so
  `--auto` had no required check to wait on. PR #80 (empty-project onboarding)
  then merged with its checks stuck pending during the same incident — a
  deliberate call, on the strength of the local gates. Local gates cover
  everything ci.yml does except the app-crate build on ubuntu, and nothing in
  this stream has been verified on Linux at all. Re-run CI on main once the
  queue is healthy, and if it keeps swallowing runs, consider whether a required
  check should gate merges at all (today nothing does).
  _From: [2026-08-06 usage-countdown session](docs/sessions/2026-08-06-usage-countdown/summary.md),
  [2026-08-06 empty-project-onboarding session](docs/sessions/2026-08-06-empty-project-onboarding/summary.md)_

- **First commit is only offered from the new-worktree form.** A tracked repo
  with an unborn HEAD gets its "Create initial commit" action when you click
  **New worktree** and nowhere else. If that state proves common, the project
  row is the more discoverable home for it.
  _From: [2026-08-06 empty-project-onboarding session](docs/sessions/2026-08-06-empty-project-onboarding/summary.md)_

- **Usage credits in the widget.** The oauth/usage endpoint's `spend` object
  carries extra-usage credits (balance, cap, severity); skipped in v0.7.0
  because credits are disabled on this account. Add a fourth row when enabled.
  _From: [2026-08-02 usage-widget session](docs/sessions/2026-08-02-usage-widget/summary.md)_

- **Multi-harness usage rows.** Widget is Claude-only by design today; when
  the app grows other harnesses, generalize to one provider per row group
  (backend already isolates the Claude source behind a single command).
  _From: [2026-08-02 usage-widget session](docs/sessions/2026-08-02-usage-widget/summary.md)_

- **App button for `worktrees init --diff`.** `doctor` now emits an
  `undeclared` finding when a gitignored file exists that `.worktrees.toml`
  never learned about, and the app already renders it (`ProjectSheet.tsx` maps
  all findings; `place: null` correctly marks no row). What's missing is the
  action beside it — Relink and Provision have buttons, this doesn't.
  _From: [2026-08-05 undeclared-drift session](docs/sessions/2026-08-05-undeclared-drift/summary.md)_

- **`doctor --strict` is credential-only for `undeclared`.** By design: an
  undeclared `.env*` is Info, so `--strict` (which promotes Warn→Error) never
  fails on it, matching how `--strict` treats `copy-stale`. If a project wants
  undeclared `.env*` to fail CI too, that knob does not exist yet.
  _From: [2026-08-05 undeclared-drift session](docs/sessions/2026-08-05-undeclared-drift/summary.md)_

- **Glob `path` inside `[[file]]` — evaluated and DECLINED, recorded so it is
  not re-litigated.** It costs the two things that make the format work:
  per-entry `mode` (link vs copy is load-bearing) and "declared but missing =
  warning". cdv's real config is 6 stanzas. Reopen only for a repo where one
  stanza per package is genuinely painful, and prefer extending `init --diff`
  even then. Same session ruled out a `.gitignore`-style `.worktreeinclude`
  file for the same reasons plus two more (security surface, and two-thirds of
  `.worktrees.toml` isn't files).
  _From: [2026-08-05 undeclared-drift session](docs/sessions/2026-08-05-undeclared-drift/summary.md)_

- **cdv migration to `.worktrees.toml`.** Still the open item from the
  project-settings proposal (§12): delete `scripts/worktrees.sh`'s stack-mode
  block in one commit, transcribe `[ports] base` from cdv `main` (not the
  proposal doc), and run `provision` against the two unprovisioned worktrees
  before anyone runs `deploy-local.sh` in either.
  _From: [docs/proposals/project-settings.md §12](docs/proposals/project-settings.md)_

- **No automated UI tests exist at all.** The mock harness makes the app
  drivable headlessly (Playwright), and three scripts already drive it to
  produce media — but nothing asserts behaviour. Both HIGH findings in the last
  two AI-profiles reviews were UI bugs, and one of them (skill toggles saved on
  `onBlur`) exists *only* on WKWebView, the engine the harness does not run —
  so a headless suite would not have caught it either. Worth pairing a spec
  suite with at least a smoke pass in a real WebKit.
  _From: [2026-08-06 ai-rules-layer session](docs/sessions/2026-08-06-ai-rules-layer/summary.md)_

- **`--` end-of-options in core's arg parsers.** ADR 0001 forbids repo-supplied
  argv; the MCP server enforces the same at its own boundary with `safe_arg`,
  because a value beginning with `-` is consumed as a flag and
  `--ai=<cmd>` reaches `ops::launch`, which interpolates it into `sh -ic`. That
  boundary check is the fix that shipped; a `--` terminator in `cmd_new` /
  `cmd_open` / `cmd_rm` would be the second layer, and would make the property
  structural rather than remembered.
  _From: [2026-08-06 ai-rules-layer session](docs/sessions/2026-08-06-ai-rules-layer/summary.md)_

- **Importing or sharing an AI profile — blocked on treating it as executable
  content.** A profile's `mcp_servers` become subprocess command lines and its
  `settings.json` is where `hooks` live. That is fine while the author is the
  user (same trust level as `ai_cmd`); it stops being fine the moment a profile
  can arrive from anywhere else, because installing one would be
  indistinguishable from running a script. Prerequisites, all of them: a
  confirmation UI showing `command`/`args`/`hooks` verbatim, default-dropping
  `hooks` and `permissions` from imported settings, and `source: "imported"`
  provenance so the UI can badge it. v1 deliberately ships "reveal the file in
  Finder" instead.
  _From: [2026-08-06 ai-rules-layer session](docs/sessions/2026-08-06-ai-rules-layer/summary.md)_

- **Keychain GC on profile delete — evaluated and DECLINED for now.** claude
  derives its keychain service name from an undocumented 8-hex hash of the
  config-dir path, so mapping a profile to its item means reimplementing that
  hash and betting it never changes. Core also has no credential code path by
  design, which is the property that made the whole feature safe. Delete
  therefore *names* what it left behind (dir + service name when one was
  recorded) rather than pretending to clean it. Reopen only if claude documents
  the derivation or exposes the item itself.
  _From: [2026-08-06 ai-rules-layer session](docs/sessions/2026-08-06-ai-rules-layer/summary.md)_

- **Nav hierarchy in light themes.** The project-header and dormant-fade work
  was verified only against dark themes (the harness runs tokyo-night; the
  complaint screenshot was catppuccin-mocha). Everything keys off tokens, but
  `.group.dormant`'s `opacity: 0.62` is a fixed number, and 62% over a light
  ground is a different perceptual step than 62% over a dark one. Wants a pass
  through tokyo-day / catppuccin-latte, and possibly a per-theme value.
  _From: [2026-08-06 nav-hierarchy session](docs/sessions/2026-08-06-nav-hierarchy/summary.md)_

- **Project names truncate sooner in the nav.** Accepted cost of moving `.pname`
  to `--fs-row`: at narrow nav widths a long repo reads `casa-del-valle-mo…`.
  The `title={pv.root}` tooltip covers it and the nav is user-resizable, so this
  was taken deliberately — but a middle-ellipsis (`casa-…-monorepo`) would keep
  the distinguishing tail, which for sibling repos sharing a prefix is the half
  that matters.
  _From: [2026-08-06 nav-hierarchy session](docs/sessions/2026-08-06-nav-hierarchy/summary.md)_

- **The Files tree re-renders every open directory on every poll tick.** The
  tree now re-lists on `placesToken` (v0.10.0), which is what makes new files
  appear — but each bump runs `setLoading(true)` → `setKids(freshArray)` →
  `setLoading(false)` per open node, handing React a new array identity even
  when the listing came back byte-identical. Roughly three renders per open node
  per 30s, all to display what is already on screen. Not a correctness bug:
  child effects key on `entry.path`, so nothing refetches. The fix is the same
  shape v0.9.1 used for `ws` — a byte-compare before `setState`, `lastSnap` in
  `App.tsx` — applied to `setKids`/`setEntries` in `FilesPane.tsx`. Worth doing
  if the dock is ever left open on a deep tree, or alongside the backend
  poll-gating item above.
  _From: [2026-08-09 files-tab-refresh session](docs/sessions/2026-08-09-files-tab-refresh/summary.md)_

- **The `.git` read path has no guard of its own.** `list_dir` now refuses to
  follow a symlink whose target resolves into a `.git` — the listing hides
  `.git` by NAME, so `ln -s .git g` walked around it, and
  `guard_under_projects` has no opinion (it accepts any existing path under a
  registered root). The READ path still does not: a markdown doc link to
  `/abs/path/.git/config` opens in the viewer via `read_file`. Pre-existing,
  not a regression, and reachable only from a link a repo supplies — which is
  exactly the threat model ADR 0001 argues about. The fix is one component
  check in `guard_under_projects`, but it is a behaviour change on every read
  command, so it wants its own change and its own test rather than a
  drive-by.
  _From: [2026-08-10 files-tab-visibility session](docs/sessions/2026-08-10-files-tab-visibility/summary.md)_

- **The workspace-containment predicate now exists twice.**
  `guard_under_projects` keeps its own inline scan so it can short-circuit on
  the first matching root (canonicalizing every root eagerly would stat a dead
  network mount ordered after the hit), while `under_roots` takes an
  already-built slice for `list_dir`'s per-symlink question. Same rule, two
  expressions of it, and the guard is the app's entire filesystem boundary —
  so a future edit to one is a silent divergence. Unifying means either giving
  up the short-circuit or threading a lazy iterator through both; neither is
  obviously worth it today, which is why this is a note and not a task.
  _From: [2026-08-10 files-tab-visibility session](docs/sessions/2026-08-10-files-tab-visibility/summary.md)_

- **A true `worktrees rename` — a CLI verb, not a UI button.** v0.12's place
  `title` renames the LABEL and deliberately leaves identity alone, because the
  slug is `basename(worktree_dir)` re-derived on every read, so renaming the
  place means renaming the directory — and that is six systems, non-atomically:
  the git worktree registration, the declared store's `BTreeMap` key (`store.rs`
  has only `edit()`, which CREATES on absent — there is no delete-key API), the
  tmux session `{prefix}-{slug}` plus every `~term` sidecar (`tmux.rs:24-27`),
  the recorded `COMPOSE_PROJECT_NAME` in `.worktree.env` (which wins forever
  once written, `provision.rs:522`), the app's slug-keyed maps (`ShellKey`,
  `term_tab_names`, `place_panels`, `manual_order`), and — the one that decides
  it — **the Claude history directory, keyed on the ABSOLUTE worktree path**
  (`project.rs:645-650`), so a rename silently orphans the conversation and
  breaks auto-resume. Any real attempt has to answer the history question FIRST;
  a verb that renames five things and quietly drops your agent transcript is
  worse than no verb. Wants a recovery path for a partial failure, too.
  _From: [2026-08-11 space-workbench session](docs/sessions/2026-08-11-space-workbench/summary.md)_

- **Mock-harness console noise, unexplained.** Switching places in
  `pnpm dev:mock` throws two kinds of `TypeError` per switch: xterm's
  `Viewport.syncScrollArea` reading `dimensions` of undefined, and the Tauri
  event shim's `_unlisten` reading `unregisterListener` of undefined. **Both are
  pre-existing** — proven by replaying the identical click sequence against a
  content-checked baseline harness (18 and 12 occurrences, same as the change
  under test), so they are not from the panel or title work. They are harness-
  only (the real backend has a real `unlisten`), which is why they have survived
  this long, but they make `browser_console_messages` noisy enough that a REAL
  error can hide among them — which is a live risk every time the harness is used
  to verify something. Worth a session: the unlisten one looks like the mock's
  `listen` returning an unlisten that assumes a plugin the browser does not have.
  _From: [2026-08-11 space-workbench session](docs/sessions/2026-08-11-space-workbench/summary.md)_

- **`term_tab_names` is not pruned at the lifecycle points `place_panels` now
  is.** Both are per-place maps in `ui-state.json` keyed `repo|slug`, but only
  `place_panels` gained a once-per-session sweep plus explicit drops on
  `remove_place` and `remove_project` (2026-08-11). `term_tab_names` still only
  prunes its own empty buckets when a tab is renamed, so a removed place leaves
  its terminal tab names behind forever. Pre-existing and harmless (a few bytes,
  and a stale key can only be read by a place with the same repo+slug, i.e. one
  recreated at the same path — where arguably you WANT the old names back), but
  the two maps now behave differently for no articulated reason, and `dropPanels`
  is a one-line generalisation away from covering both.
  _From: [2026-08-11 space-workbench session](docs/sessions/2026-08-11-space-workbench/summary.md)_

- **The side-by-side diff has never been run in the real app.** It was built and
  verified entirely against the mock harness plus direct git probes, and
  CLAUDE.md is explicit that the mock cannot express real timing — its invokes
  resolve in a microtask, where `file_diff` is a git spawn per file on top of the
  `changed_files` fan-out. One bug already found in review lived exactly there
  (the changed-only filter asserting "nothing changed on this branch" during the
  window before the change set lands, which the mock has no window for), so the
  class is not hypothetical. `sandbox.sh --app` cannot be driven from an agent —
  it `exec`s `tauri dev` into a native WKWebView and its scratch repo has no
  changed files to diff — so this wants a human pass on a real repo before the
  next release: a large file, a rename, a binary, and the diff open while a
  `git checkout` changes the tree underneath it.
  _From: [2026-08-18 side-by-side-diff session](docs/sessions/2026-08-18-side-by-side-diff/summary.md)_

- **The diff has no manual side-by-side / unified pin.** `DiffView` picks by
  measured body width (`DIFF_SIDE_AT`, 560px) and that is the whole rule, where
  the tree beside it gives `files_layout` an explicit auto → stacked → side-by-
  side cycle. The two are the same question asked about different panes, so a
  wide dock that the user nonetheless wants read as one column has no answer.
  Deliberately left out of the first slice rather than forgotten: a second
  layout control in the same header, next to a Preview/Source/Diff seg and a
  Base/HEAD seg, needs a reason better than symmetry.
  _From: [2026-08-18 side-by-side-diff session](docs/sessions/2026-08-18-side-by-side-diff/summary.md)_

- **The diff's add/del edge is under 3:1 on two light themes.** Measured against
  the code surface, `box-shadow: inset 2px` runs 7:1–12:1 on the four dark
  themes but ~2.4:1 for deletions on Tokyo Day and additions on Catppuccin Latte,
  where `--ok`/`--danger` are darkened for a light nav. It is a supporting cue —
  position carries side-by-side and a sign column carries unified — so nothing is
  unreadable, but a light-theme user gets one channel fewer than a dark-theme
  one. Wants either per-appearance diff tokens or a wider edge on light themes.
  While measuring it: **`--syn-com` is ALREADY below 4.5:1 against `--bg-abyss`
  on all four dark themes** (Nord 3.13, gruvbox-dark 3.52, Catppuccin Mocha 4.02,
  Tokyo Night 4.09) — pre-existing and unrelated to the diff, but `tokens.css`
  claims the ramp is ≥ 4.5:1 and that comment is only true of the light pair.
  _From: [2026-08-18 side-by-side-diff session](docs/sessions/2026-08-18-side-by-side-diff/summary.md)_

- **The nav's dirty count and the tree's change badge answer different
  questions, and nothing on screen says so.** `dirty_files` (the nav's badge,
  from `status_v2` in `project.rs`) counts UNCOMMITTED files only; a directory
  badge in the Files tab counts uncommitted *and* committed-on-this-branch. Both
  are right for their own surface — "is there work I could lose" vs "what did
  this branch touch" — but a place showing no dirty badge next to a tree full of
  marks looks like a bug, and the only place the distinction is written down is
  the CHANGELOG entry and the session summary. Either a tooltip on each side
  saying which question it answers, or the nav grows a second (quieter) signal
  for branch divergence by FILE — it already shows it by commit (↑↓).
  _From: [2026-08-11 files-changed-markers session](docs/sessions/2026-08-11-files-changed-markers/summary.md)_

- **The changed-file markers have no off switch.** Nobody asked for one, and the
  bar for new config in this repo is deliberately high (ADR 0001), so they are
  simply always on — unlike the Files tab's other two behaviours (show-ignored,
  layout), which are both persisted toggles. If the tint ever proves too loud
  next to the gitignored dimming and the symlink `↗`, the settings shape already
  exists (`files_show_ignored` is the pattern to copy) and the cost of the git
  calls is the thing an off switch would actually save.
  _From: [2026-08-11 files-changed-markers session](docs/sessions/2026-08-11-files-changed-markers/summary.md)_

- **The mock harness cannot express real backend timing, and three shipped bugs
  came out of that gap in one session.** `list_workspace` answers in a microtask
  there, so: two sweeps never overlap (an ordering race cannot exist), there is
  no gap between "write done" and "refresh returned" (a latency bug cannot
  exist), and any state that only exists *before* a record is written is easy to
  never exercise. All three v0.12.x bugs — the dock seeding from the last place,
  the rename not reaching the nav, and the stale sweep reverting it — passed
  gates, adversarial review and harness verification, and were then found by
  running the real app via `app/scripts/sandbox.sh --app`. Two narrow
  mitigations shipped: `?slowlist=<ms>` in `app/src/mock/install.ts` (real
  sweep latency) and `app/scripts/race-check.mjs` (controlled promise-resolution
  orders over the real `refresh`/`commitWs`/`patchDeclared`/`mutate` source).
  The general problem is untouched: the mock is a fixture store with instant
  IPC, and anything whose correctness lives in the timing between calls is
  invisible to it. Worth considering a mode where every mocked invoke takes a
  configurable, jittered delay by default, so the harness is timing-hostile
  rather than timing-free — the current default of "instant" is the one setting
  guaranteed never to catch this class.
  _From: [2026-08-11 v0-12-releases session](docs/sessions/2026-08-11-v0-12-releases/summary.md)_

- **`patchDeclared` covers title, pin and note but not lifecycle**, so changing
  a place's lifecycle still waits on a full `list_workspace` sweep — seconds on
  a large workspace, the same lag the other three used to have. Deliberate:
  `lifecycle_effective` is reconciled server-side from the declared label AND
  live tmux state (`store::reconcile`), so patching the label alone would show a
  row disagreeing with its own badge. Closing it means either reimplementing
  `reconcile` in TypeScript (a second source of truth for a rule that already
  bit this repo once) or having `set_lifecycle` return the reconciled place so
  the frontend can patch from the backend's own answer. The second is the right
  shape and is a small command-signature change.
  _From: [2026-08-11 v0-12-releases session](docs/sessions/2026-08-11-v0-12-releases/summary.md)_

- **`refresh()` re-snapshots every registered project on every call**, and it is
  called from eight places. Measured 0.28s for one project with nine worktrees;
  a real multi-project workspace is seconds, and it runs on the 3s poll, on
  every `places:changed`, after every command and on the visible edge. The
  ordering guard added in #118 stops stale reads from *winning*, but the work is
  still done and thrown away. A per-project refresh (the changed root only)
  would cut nearly all of it — `list_places(repo)` already exists and returns
  exactly one project's snapshot; nothing calls it from the poll path.
  _From: [2026-08-11 v0-12-releases session](docs/sessions/2026-08-11-v0-12-releases/summary.md)_

- **`make install-app` cannot complete unattended.** It builds and bundles fine,
  then dies signing the updater artifact: `~/.tauri/worktrees-updater.key` is
  password-protected and tauri prompts for it on a TTY, so any non-interactive
  run (an agent, a script, CI-on-a-Mac) aborts *after* a full cargo build and
  *before* the `ditto` into /Applications — a successful build with nothing
  installed. Exporting `TAURI_SIGNING_PRIVATE_KEY` does not help; the password
  prompt remains. A local install has no use for the updater artifact at all, so
  the fix is either a `--no-bundle`-style path (or `createUpdaterArtifacts` off)
  for the install-app target, or an unencrypted key kept out of the repo for
  local use only. Today the workaround is running the `rm -rf` + `ditto` steps
  by hand after the bundle lands.
  _From: [2026-08-02 remove-place-delbranch session](docs/sessions/2026-08-02-remove-place-delbranch/summary.md)_
