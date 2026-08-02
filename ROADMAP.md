# Roadmap

Parking lot for work we've decided to keep but not do now. Each item links
the session summary that spawned it (see docs/sessions/). Groomed during the
close-out ritual (.claude/skills/close-out).

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
  - _Test infra:_ mock fault-injection (no mock `CmdResult` returns `ok:false`, so
    the error-banner path is untestable headlessly).
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

- **Dock file viewer: syntax highlighting.** The editable viewer is a plain
  mono `<textarea>` (deliberately — the project bans UI libraries). If in-app
  editing becomes a real workflow rather than a quick peek, revisit: a light
  highlighter for read mode, or CodeMirror for edit mode (which would be the
  first real UI-lib dependency — a decision, not a default).
  _From: [2026-07-28 right-dock session](docs/sessions/2026-07-28-right-dock/summary.md)_

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
