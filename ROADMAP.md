# Roadmap

**Open work lives in [GitHub issues](https://github.com/penard-monkey/worktrees/issues).**
This file is no longer the parking lot. It is the index over those issues, plus
the things that are deliberately *not* issues — decisions already made, notes
waiting on evidence, and chores that belong to a machine rather than the repo.

Groomed during the close-out ritual (global `/close-out` skill; this repo's
settings in `.claude/close-out.md`).

## How to use it

- **Picking something up?** Start with
  [`good first issue`](https://github.com/penard-monkey/worktrees/labels/good%20first%20issue)
  — self-contained, no special hardware, the local gates in CLAUDE.md are the
  whole bar.
- **Filtering by area:** `area:app` · `area:core` · `area:cli` ·
  `area:ci-tooling` · `area:docs`.
- **`needs-real-mac`** means it can only be confirmed by hand in the real app on
  macOS. There is no fake `claude` and no fake tmux, the harness is Chrome and
  the app is WKWebView, and Playwright speaks CDP while the sandbox is a native
  window. Those are not contributor tasks and they cannot be automated.
- **`tech-debt`** is debt recorded on purpose, not a defect. Read the issue
  before "fixing" it; several say why the obvious fix is wrong.

Every issue carries the full prose from the session that spawned it and links
that session's summary under [docs/sessions/](docs/sessions/).

---

## Index

### CI, gates and the harness

| # | |
| --- | --- |
| [#228](https://github.com/penard-monkey/worktrees/issues/228) | Guard against a second `## [Unreleased]` header |
| [#229](https://github.com/penard-monkey/worktrees/issues/229) | Wire the static check scripts into CI |
| [#230](https://github.com/penard-monkey/worktrees/issues/230) | A drift guard for the Escape stack (`esc-check.mjs`) |
| [#231](https://github.com/penard-monkey/worktrees/issues/231) | A reusable WebKit probe as a check script |
| [#232](https://github.com/penard-monkey/worktrees/issues/232) | Nothing checks the new-worktree verdict against `ops.rs` |
| [#233](https://github.com/penard-monkey/worktrees/issues/233) | Flaky: `skills add installs from a local git repo` |
| [#234](https://github.com/penard-monkey/worktrees/issues/234) | Make the mock harness timing-hostile rather than timing-free |
| [#235](https://github.com/penard-monkey/worktrees/issues/235) | Mock harness throws two TypeErrors per place switch |
| [#236](https://github.com/penard-monkey/worktrees/issues/236) | Mock harness gaps: fault injection, state machine, no-origin fixture |
| [#237](https://github.com/penard-monkey/worktrees/issues/237) | No automated UI tests exist at all |
| [#238](https://github.com/penard-monkey/worktrees/issues/238) | Decide whether a required status check should gate merges |

### Core engine

| # | |
| --- | --- |
| [#239](https://github.com/penard-monkey/worktrees/issues/239) | `remove_place`'s `force` is two permissions wearing one flag |
| [#240](https://github.com/penard-monkey/worktrees/issues/240) | `remote_only` on `BranchList`, so the verdict can say "tracking" |
| [#241](https://github.com/penard-monkey/worktrees/issues/241) | `do_switch` still pays the doomed two-fetch pair |
| [#242](https://github.com/penard-monkey/worktrees/issues/242) | `--` end-of-options in core's arg parsers |
| [#243](https://github.com/penard-monkey/worktrees/issues/243) | Existing stores never get the `.git/info/exclude` entries |
| [#244](https://github.com/penard-monkey/worktrees/issues/244) | `DirLock` uses mtime staleness, contradicting DESIGN.md |
| [#245](https://github.com/penard-monkey/worktrees/issues/245) | The sync heal matcher skips patterns rsync would anchor |
| [#246](https://github.com/penard-monkey/worktrees/issues/246) | The `.git` read path has no guard of its own |
| [#247](https://github.com/penard-monkey/worktrees/issues/247) | The workspace-containment predicate now exists twice |
| [#248](https://github.com/penard-monkey/worktrees/issues/248) | A true `worktrees rename` — a CLI verb, not a UI button |
| [#249](https://github.com/penard-monkey/worktrees/issues/249) | Doctor's skipped-files scan: hot-path cost, deletion vs damage |
| [#250](https://github.com/penard-monkey/worktrees/issues/250) | Nothing pins tmux's `base-index`, and the app assumed it |
| [#286](https://github.com/penard-monkey/worktrees/issues/286) | Sync v2: SSH targets, push-all, scheduling |
| [#292](https://github.com/penard-monkey/worktrees/issues/292) | Agent workflow, next slices |

### App — correctness and cost

| # | |
| --- | --- |
| [#251](https://github.com/penard-monkey/worktrees/issues/251) | A global React error boundary |
| [#252](https://github.com/penard-monkey/worktrees/issues/252) | The nav is rebuilt, not re-rendered |
| [#253](https://github.com/penard-monkey/worktrees/issues/253) | PTY → IPC coalescing (terminal throughput) |
| [#254](https://github.com/penard-monkey/worktrees/issues/254) | The backend 3 s poll loop is the largest background cost |
| [#255](https://github.com/penard-monkey/worktrees/issues/255) | Event-driven place refresh (FSEvents) |
| [#256](https://github.com/penard-monkey/worktrees/issues/256) | `refresh()` re-snapshots every project on every call |
| [#257](https://github.com/penard-monkey/worktrees/issues/257) | `set_lifecycle` should return the reconciled place |
| [#258](https://github.com/penard-monkey/worktrees/issues/258) | The Files tree re-renders every open directory on every tick |
| [#259](https://github.com/penard-monkey/worktrees/issues/259) | A dock tab's identity is split across two files |
| [#260](https://github.com/penard-monkey/worktrees/issues/260) | The tmux pane drops replies to attach-time queries |
| [#261](https://github.com/penard-monkey/worktrees/issues/261) | The dev/sandbox build logs into the INSTALLED app's `app.log` |
| [#262](https://github.com/penard-monkey/worktrees/issues/262) | Zombie children — real, unreproduced, obvious diagnosis wrong |
| [#263](https://github.com/penard-monkey/worktrees/issues/263) | The column floors are px constants, and page zoom moves them |

### App — interface

| # | |
| --- | --- |
| [#264](https://github.com/penard-monkey/worktrees/issues/264) | Links in release notes still print raw |
| [#265](https://github.com/penard-monkey/worktrees/issues/265) | Filter Enter misses matches inside a collapsed group |
| [#266](https://github.com/penard-monkey/worktrees/issues/266) | Two nav drag nits: padding drops nothing, ghost re-renders App |
| [#267](https://github.com/penard-monkey/worktrees/issues/267) | Nav polish: disabled tooltips, truncation, light-theme fade |
| [#268](https://github.com/penard-monkey/worktrees/issues/268) | The diff's add/del edge is under 3:1 on two light themes |
| [#269](https://github.com/penard-monkey/worktrees/issues/269) | The diff has no manual side-by-side / unified pin |
| [#270](https://github.com/penard-monkey/worktrees/issues/270) | Dirty count and change badge answer different questions |
| [#271](https://github.com/penard-monkey/worktrees/issues/271) | Markdown viewer gaps |
| [#272](https://github.com/penard-monkey/worktrees/issues/272) | App button for `worktrees init --diff` |
| [#273](https://github.com/penard-monkey/worktrees/issues/273) | Two unrelated things are now called "Claude" in Settings |
| [#274](https://github.com/penard-monkey/worktrees/issues/274) | Usage widget: polled-ago line, credits row, multi-harness rows |
| [#275](https://github.com/penard-monkey/worktrees/issues/275) | Global summon hotkey |
| [#276](https://github.com/penard-monkey/worktrees/issues/276) | Note keystrokes land in the shell on a non-selected place |
| [#277](https://github.com/penard-monkey/worktrees/issues/277) | `github-url` does not percent-encode the branch name |
| [#278](https://github.com/penard-monkey/worktrees/issues/278) | Nav / settings audit backlog (2026-07-27) |
| [#279](https://github.com/penard-monkey/worktrees/issues/279) | Right-panel nits: `--nav-w`/`--dock-w`, combobox Enter clamp |
| [#280](https://github.com/penard-monkey/worktrees/issues/280) | Find is deliberately narrow in three places |
| [#281](https://github.com/penard-monkey/worktrees/issues/281) | Dock file viewer: editing, and the highlighter's gaps |
| [#282](https://github.com/penard-monkey/worktrees/issues/282) | First commit is only offered from the new-worktree form |
| [#287](https://github.com/penard-monkey/worktrees/issues/287) | Importing or sharing an AI profile |

### Distribution, CLI and docs

| # | |
| --- | --- |
| [#283](https://github.com/penard-monkey/worktrees/issues/283) | App signing + notarization (Developer ID) |
| [#284](https://github.com/penard-monkey/worktrees/issues/284) | `make install-app` cannot complete unattended |
| [#285](https://github.com/penard-monkey/worktrees/issues/285) | tmux gate: an unrun interactive path, unbounded PATH growth |
| [#288](https://github.com/penard-monkey/worktrees/issues/288) | Work-stream framing sweep |
| [#289](https://github.com/penard-monkey/worktrees/issues/289) | Regenerate the README media |
| [#290](https://github.com/penard-monkey/worktrees/issues/290) | Remove the MCP resource debug logging (gate: 0.26.0) |
| [#291](https://github.com/penard-monkey/worktrees/issues/291) | Two known edges in the MCP resource URIs |

### Verification debt — `needs-real-mac`

| # | |
| --- | --- |
| [#293](https://github.com/penard-monkey/worktrees/issues/293) | The ✎ suggestion filter is coupled to Claude Code's rendering |
| [#294](https://github.com/penard-monkey/worktrees/issues/294) | Real-app verification debt — shipped on mock evidence only |
| [#295](https://github.com/penard-monkey/worktrees/issues/295) | `docs/ai-profiles-manual-checks.md` has never been fully run |

---

# Not issues

These are kept here on purpose. Filing them would put a task in front of
something that is not a task.

## Decided and declined

Recorded so they are not re-litigated. Each names the condition that would
reopen it.

- **Glob `path` inside `[[file]]` — DECLINED.** It costs the two things that
  make the format work: per-entry `mode` (link vs copy is load-bearing) and
  "declared but missing = warning". cdv's real config is 6 stanzas. Reopen only
  for a repo where one stanza per package is genuinely painful, and prefer
  extending `init --diff` even then. The same session ruled out a
  `.gitignore`-style `.worktreeinclude` file for the same reasons plus two more
  (security surface, and two-thirds of `.worktrees.toml` isn't files).
  _From: [2026-08-05 undeclared-drift](docs/sessions/2026-08-05-undeclared-drift/summary.md)_

- **The Mac App Store is not a distribution route — DECLINED.** Full reasoning
  in [#283](https://github.com/penard-monkey/worktrees/issues/283): MAS requires
  App Sandbox, which forbids shelling out to `git`/`tmux`, puts the tmux server
  in a container where neither the CLI nor the user's shell can attach, and
  prohibits the self-updater. Sandboxing would "fix" the privacy prompt by
  making the access impossible.

- **Keychain GC on profile delete — DECLINED for now.** Detail in
  [#287](https://github.com/penard-monkey/worktrees/issues/287). claude derives
  its keychain service name from an undocumented 8-hex hash of the config-dir
  path. Reopen only if claude documents the derivation or exposes the item.

- **`doctor --strict` is credential-only for `undeclared` — by design.** An
  undeclared `.env*` is Info, so `--strict` (which promotes Warn→Error) never
  fails on it, matching how `--strict` treats `copy-stale`. If a project wants
  undeclared `.env*` to fail CI too, that knob does not exist yet — but nobody
  has wanted it.
  _From: [2026-08-05 undeclared-drift](docs/sessions/2026-08-05-undeclared-drift/summary.md)_

## Parked with a reason

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

- **A layout change while a shell tab is detached still replays at the wrong
  width.** PR #153 stops the pane attaching at a size that isn't its own, which
  was the reproducible case — but a ring written at one width and replayed
  after a REAL change (⌘B while flipped away, a window resize) is still raw
  bytes laid out for a grid that no longer exists, and no byte log replays
  faithfully across that. Ages out via the 256K cap. The full fix is
  terminal-state serialization (a server-side screen model, replay state not
  bytes) — parked because the degraded case is now rare and self-healing.
  _From: [2026-08-17 gitignore-cmdt-replay](docs/sessions/2026-08-17-gitignore-cmdt-replay/summary.md)_

## Waiting on evidence

Notes, not tasks. Each needs data or lived experience before anyone should act.

- **Read Settings → Usage after two weeks, then cut.** The lenses went by
  decision, not data; the next round (project headers under a filter, the sort
  menu, Home's resume list, the dock rail, `PanelRightClose/Open` icons that
  nothing imports) should go by the heatmap. Install first — the metrics only
  start with a build that has them.
  _From: [2026-09-05 ui-overhaul](docs/sessions/2026-09-05-ui-overhaul/summary.md)_

- **Usage heatmap blind spots.** No key, so invisible: combobox popup rows
  (`.combo-item` — branch switcher, new-worktree base picker), context-menu
  `pop-item`s without a `title`, non-button click targets (`.sb-label`, the
  Claude-usage chip rows, `.viewer-tag`, StatusSheet `Row`), two over-long static
  titles (FilesPane "Show what this branch changed…", ProjectSheet "re-seed
  declared copies…"). Coarsening: `closest("[data-testid]")` keys an untitled
  control inside the sync/import modals as `sync-modal`/`import-modal`. Fix with
  `data-track` where a row shows up dark for the wrong reason; leave the rest
  until the data asks.
  _From: [2026-09-05 ui-overhaul](docs/sessions/2026-09-05-ui-overhaul/summary.md)_

- **Cut the usage-meter placements David does not keep.** Strip / Footer / Rail
  shipped together so he can live with each; once one wins, remove the others
  (keep `Off`) and the `.statusbar` row can go for good if Footer loses.
  _From: [2026-09-07 usage-meter-home](docs/sessions/2026-09-07-usage-meter-home/summary.md)_

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

- **The afterglow's decay is a setting now (v0.23.0); its ranges are a guess.**
  Settings → Navigation → Afterglow: horizon from `DONE_HORIZONS` (1h..7d) and
  2–6 steps, geometric from a pinned 15m first boundary, defaults reproducing
  the old 12h / 3 steps. Two soft edges: 2h × 6 steps packs three boundaries
  into 40 minutes (legal, barely distinguishable), and `snapHorizon`'s NaN
  fallback hardcodes the index of the default rather than reading `DEFAULTS`
  (import cycle). Revisit both once David has lived with a longer horizon.
  _From: [2026-09-09 afterglow-decay-setting](docs/sessions/2026-09-09-afterglow-decay-setting/summary.md)_

- **The changed-file markers have no off switch.** Nobody asked for one, and the
  bar for new config in this repo is deliberately high (ADR 0001), so they are
  simply always on — unlike the Files tab's other two behaviours (show-ignored,
  layout), which are both persisted toggles. If the tint ever proves too loud
  next to the gitignored dimming and the symlink `↗`, the settings shape already
  exists (`files_show_ignored` is the pattern to copy) and the cost of the git
  calls is the thing an off switch would actually save.
  _From: [2026-08-11 files-changed-markers](docs/sessions/2026-08-11-files-changed-markers/summary.md)_

- **⌘K lists `(main)` places; the Recent lens filters them out.** Both survived
  the ordering unification untouched, so the asymmetry is now the only thing
  left that makes those two lists disagree. Arguably right — the switcher is for
  jumping anywhere, Recent is for resurfacing work — but it has never actually
  been decided. Worth one deliberate call rather than leaving it as an artifact
  of two different filters.
  _From: [2026-08-11 cmdk-activity-order](docs/sessions/2026-08-11-cmdk-activity-order/summary.md)_

- **A just-created worktree wears the base tip's age** — "5d" seconds after
  creation, until work or a commit lands in it. Accepted, but the blast radius
  grew: this used to be a nav-tree quirk, and now that ⌘K, the Recent lens and
  the home Resume list all rank on `activityAt`, a fresh place sits wherever its
  base branch's last commit puts it in **every** list. The fix, if it ever reads
  wrong in practice, is a creation epoch in the declared store — not a special
  case in the clock.
  _From: [2026-08-10 nav-activity-age](docs/sessions/2026-08-10-nav-activity-age/summary.md),
  widened [2026-08-11 cmdk-activity-order](docs/sessions/2026-08-11-cmdk-activity-order/summary.md)_

- **Does a parked `waiting` probe pin the amber dot too?** The park-residue fix
  (#140) guards the `busy` arm of `claude_activity` only. If a session can be
  parked while its status is `waiting`, the same stale-status residue would pin
  an amber "needs input" dot forever — one word to fix (`"waiting" if
  !delegated`), but no probe on this machine has ever shown that shape and it is
  unknown whether the CLI even allows parking from a blocked prompt. **Find out
  before adding the guard; a wrong guess darkens a dot that should be lit.**

## Local chores — not repo work

These belong to a machine, not to the codebase. They are here so they are not
forgotten, not so someone picks them up.

- **The `mcp-setup-wizard` worktree's idle base is the bare tree name.** Every
  other tree parks on `<tree>-next` (`.claude/close-out.md`); this one was created
  on a branch called `mcp-setup-wizard`, which is the shape the config exists to
  prevent — two worktrees sharing a base produce the phantom-staged-state failure
  recorded in that file, and a bare tree name is the most likely collision.
  Rename to `mcp-setup-wizard-next` (nothing depends on the old name; no PR has
  ever targeted it).

- **Stale worktree/branch cleanup.** `New-icon` and `spike-demo` worktrees (and
  their branches) predate the current stream; decide merge/abandon and remove.
  `release-v0.2.3` is superseded by v0.2.4. Plus a pile of merged/abandoned
  REMOTE branches from earlier streams still on origin (`feat/app-install`,
  `feat/app-self-update`, `feat/in-app-update`, `feat/nav-prefs`,
  `feat/ui-polish`, `feat/ui-redesign`, `fix/gui-path`,
  `fix/term-resize-artifacts`, `fix/ui-responsiveness`, `next`,
  `release/0.2.0|0.2.1|0.2.2`, `docs/*`, `chore/*`, `settings-next`) — squash
  merges break `git branch --merged`, so verify each against its PR before
  `git push origin --delete`.

- **`~/bin/sync-macs` still exists.** Its retirement gate passed at phase 1;
  David deletes it manually. (Tracked alongside
  [#286](https://github.com/penard-monkey/worktrees/issues/286).)

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
  The remaining `.worktrees.toml` step (proposal §12): delete
  `scripts/worktrees.sh`'s stack-mode block in one commit, transcribe
  `[ports] base` from cdv `main` (not the proposal doc), and run `provision`
  against the two unprovisioned worktrees before anyone runs `deploy-local.sh`.
  _From: [2026-07-28 project-settings](docs/sessions/2026-07-28-project-settings/summary.md),
  [docs/proposals/project-settings.md §12](docs/proposals/project-settings.md)_

- **Project-settings polish leftovers** — too small to file, kept for whoever is
  next in those files:
  - `app/src/mock/install.ts`'s `SUGGESTED_TOML` is a hand-written fixture, not
    a mirror of `init::render()`, so the harness preview lacks init's warnings.
    No user-facing drift (production uses the real emitter).
  - A refresh-raised error is retracted by identical-string match; two sources
    producing byte-identical text would cross-clear.
  - The 4s arm auto-disarm is tight for "Kill &lt;session&gt; — whole session?".
  - `doctor`'s session-drift scan skips `(main)` on named-place runs.
  - Spec §4 still says "the same rules apply to `[compose] file`" — true per
    entry, but the key is `files` now.
  _From: [2026-07-28 project-settings](docs/sessions/2026-07-28-project-settings/summary.md)_
