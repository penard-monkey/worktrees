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
  `git push origin --delete`. (This session's 9 branches were cleaned at close-out.)
  _From: [2026-07-26 themes session](docs/sessions/2026-07-26-themes-v0.2.4/summary.md)_

- **Cut v0.3.2** — `[Unreleased]` holds the ⌘K quick switcher (#52). Bundle the
  global-summon-hotkey with it (below) before cutting, then follow CLAUDE.md
  release steps. (v0.2.5 was superseded — the accumulated work shipped as
  v0.3.0 + v0.3.1.)
  _From: [2026-07-27 settings session](docs/sessions/2026-07-27-settings-and-audits/summary.md)_

- **Per-project settings (`.worktrees.toml`)** — design decided, nothing built;
  build slot is **after v0.3.2**. Full spec: `docs/proposals/project-settings.md`.
  v1 = `[[file]]` link/copy + `[ports]`/`.worktree.env` + `[compose]` namespacing
  and teardown + `relink`/`provision`/`doctor`. Generic by design — each section
  stands alone; `casa-del-valle-monorepo` is the first consumer, not the spec.
  Decided: no repo-supplied argv ever (no `[hooks]`, and `DESIGN.md:225-228`'s
  `[infra] up/stop/down` is reversed — the tool assembles the compose argv from
  data); TOML for both human-authored config files; port slot derived from
  `<wt>/.worktree.env`, never stored in `.worktrees.places.json`.
  ⚠ Carries a live hazard: two cdv worktrees created by this tool have no
  `.worktree.env`, and that repo's `deploy-local.sh` treats its absence as
  "not a worktree" → global `pkill -9` that kills the main checkout's stack.
  Prereqs that must land with it: `.worktree.env` in `ensure_excluded` (else
  `switch`/`rm` refuse forever and the GUI can't pass `--force`), and severity
  in `CaptureUi` (warnings are currently discarded at capture).
  _From: 2026-07-27 project-settings design session_

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
