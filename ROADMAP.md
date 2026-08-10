# Roadmap

Parking lot for work we've decided to keep but not do now. Each item links
the session summary that spawned it (see docs/sessions/). Groomed during the
close-out ritual (.claude/skills/close-out).

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
  _From: [2026-07-26 themes session](docs/sessions/2026-07-26-themes-v0.2.4/summary.md)_

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
  liveness are the untested-in-prod paths).
  _From: [2026-07-27 settings session](docs/sessions/2026-07-27-settings-and-audits/summary.md)_

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
  still written by App's layout effect but nothing consumes them (grid uses
  inline px) — delete the effect or re-point the static `.app` rule;
  `place_session_cwd` doc comment still explains sidecar-name stability;
  combobox Enter with a highlight index stale after a filter shrink falls back
  to raw text (acceptable, but a bounds-clamp on `hi` is one line).
  _From: [2026-07-29 right-panel session](docs/sessions/2026-07-29-right-panel/summary.md)_

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
