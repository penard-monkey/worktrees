# Changelog

All notable changes to this project are documented here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/).

## [Unreleased]

## [0.3.2] - 2026-07-28

### Added
- Quick switcher: press ⌘K anywhere — even with the terminal focused — to
  fuzzy-jump to any place across every project. Type to filter (matches slug,
  branch, project, or note), arrow keys to move, Enter to jump, Esc to close;
  open it with no query and it lists your most recent places. Works with the
  nav collapsed, and each row shows the same working/needs-input dot as the nav.

## [0.3.1] - 2026-07-27

### Fixed
- The green "working" dot now tracks whether Claude is actually working, not
  whether you recently touched the session. It read a tmux activity timestamp
  that only moves when you attach or type — so it would fade a few seconds after
  you looked away even while Claude kept going, and it lit up for plain shell
  typing. It now reads Claude's own per-session state: a green blinking dot while
  Claude is working, a steady amber dot when it's waiting on you (a permission or
  dialog prompt), and nothing when it's idle — on the nav row, the project
  folder, and the Home screen alike.

## [0.3.0] - 2026-07-27

The settings release: a Settings pane that finally does what its controls say,
keyboard shortcuts that actually fire, and a batch of silent failures made loud.

### Added
- Nav: a Home entry at the top (app logo + one-click "Open a project" on the
  Home screen), folder icons on project rows, deeper nesting with more
  generous indent, and a right-aligned age column on place rows.
- System theme: pick WHICH light/dark pair "System (match macOS)" flips
  between (Settings → Theme → Light ↔ dark pair).
- Keyboard shortcuts: ⌘, opens Settings, ⌘1 jumps Home, ⌘2 / ⌘3 / ⌘4 jump to
  Places / Recent / Attention (keyboard selection always reveals the nav,
  never collapses it), and ⌘E opens the current selection in your editor. A
  read-only Shortcuts section in Settings lists every one of them.
- Settings → Commands: a "Resume Claude conversation on open" toggle. Turn it
  off and a single click opens a fresh session; right-click then offers "Open
  with resume" for the times you want to pick up where you left off. The
  effective AI command and resume argument are shown read-only, with a
  "Reveal config file" button — the config is shared with the CLI.
- Settings → Git: "Auto-fetch origin" (Off / 5 / 15 / 60 min) keeps every
  project's ahead/behind counts and the Attention lens fresh in the
  background, hardened so a credential prompt can never hang the app. A
  "Fetch origin" right-click verb on projects does the same on demand.
- Settings → Commands: an external terminal command with a `{session}` token
  (e.g. `ghostty -e tmux attach -t {session}`) adds "Open in terminal app" to
  a place's right-click menu — hidden until you configure it.
- Settings → Startup: "Restore last place on launch" reselects the place you
  left off on (it selects, nothing more — it never auto-starts a session).
- Settings → Version: a "Release notes" button reopens the notes sheet on
  demand, showing the full released history (not just the unseen slice), and a
  "Check for updates at launch" toggle.
- Settings → Logs: "Copy diagnostics" — one offline click assembles the app
  and CLI versions, the GUI's real resolved PATH, git/tmux locations, your
  effective AI config, and the last 200 log lines, ready to paste into a bug
  report.
- Settings → Data: reveal the settings file in Finder, and a two-click "Reset
  to defaults".
- Removing a worktree now offers "Confirm remove + branch" alongside the plain
  remove. Branch deletion uses git's merged-only guard (`git branch -d`), so
  it can never throw away unmerged work.

### Changed
- "What's new" renders formatted release notes — version headers, colored
  Added/Changed/Fixed tags, unwrapped bullets, `code` spans — instead of the
  raw changelog markdown.
- The green dot now means one thing: this session is working right now (tmux
  output within the last few seconds). Idle-but-open sessions show no dot; a
  busy place also badges its project's folder icon. The purple ✦ "AI session"
  glyph is gone — it was true for nearly every place, so it said nothing.
- The topbar remove action now reads "Remove worktree…" (was "Remove
  place…"), matching the right-click menu.

### Fixed
- The "Window default" size inputs did nothing — they were saved but never
  applied to a window. Removed.
- "Remove from workspace" could fire with zero confirmation from two different
  surfaces; both now arm on the first click and remove on the second. A
  subtler leak also let an armed "Confirm remove?" survive closing the ⋯ menu
  and then fire on a single click much later — that's fixed too.
- Copy actions failed silently — a stale clipboard with no signal that
  anything went wrong. Copy failures now surface.
- Enter could quietly do nothing. A tmux session that failed to start reported
  success everywhere (UI, exit code, and log alike), and a session running
  under a non-canonical name made a live place read as down so its terminal
  never mounted. Both are now loud and visible.
- Creating a worktree for a branch that already lived in another place could
  select a place that didn't exist, leaving the pane blank. The app now
  selects the place the engine actually used.
- Creation and switch failures used to show progress-looking lines instead of
  git's real complaint; git's actual reason now reaches the error banner and
  the log.
- Editor commands containing spaces or quotes (`open -a "Visual Studio Code"`)
  now work, and the new terminal command uses the same quoting.

## [0.2.4] - 2026-07-26

### Added
- Themes: the Settings theme picker grows from one option to seven — System
  (follows macOS light/dark), Tokyo Night (the existing default), Tokyo Night
  Day (the new light mode), Catppuccin Mocha, Catppuccin Latte, Nord, and
  Gruvbox Dark. Every palette uses the official upstream colors,
  contrast-verified for readability on every surface, and the embedded
  terminal recolors to match, including a full per-theme ANSI palette so
  colored terminal output stays legible on light backgrounds.

## [0.2.3] - 2026-07-26

### Fixed
- Embedded terminal drew `…`, `✻`, spinners, and other non-ASCII glyphs as
  underscores: the app attached tmux without a UTF-8 locale (GUI apps get
  launchd's bare environment), so tmux deemed the client non-UTF-8 and
  substituted `_` for every cell without an ACS line-drawing fallback. The
  embedded client now attaches with `tmux -u`, and the app sets a UTF-8
  `LANG` at startup when none is present (also covers the tmux server when
  the app is the first tmux invocation). Reopen embedded panes to pick it
  up — session content was never corrupted.

## [0.2.2] - 2026-07-26

### Added
- Nav preferences: show/hide the Active / Idle / Dormant tiers (Settings →
  Nav tiers), and a sort control in the nav header — Last used, A–Z, or
  Manual with drag-to-reorder (order remembered per project).
- Release notes on update: the first launch of a new version shows a
  "What's new" sheet with the changes since the version you were on
  (offline — the changelog ships inside the app).

### Fixed
- Embedded terminal artifacts: tmux sized windows to the SMALLEST attached
  client and only redrew that region, leaving stale "undeletable"
  characters when a bare `tmux attach` ran alongside the app. Sessions now
  use `window-size latest` + `aggressive-resize` (session-scoped; your
  global tmux config is untouched).
- CI actions bumped off deprecated Node 20.

## [0.2.1] - 2026-07-25

The self-updating release: from here on, updates are one click inside the app.

### Added
- **App self-update**: releases ship minisign-SIGNED app bundles + a
  `latest.json` updater manifest; Settings → Version gains "Update app → vX"
  (verify → download → swap → relaunch) next to the existing "Update CLI"
  button. The ⚙ badge covers both.
- The curl installer now OFFERS the desktop app on macOS (`worktrees.app` →
  /Applications, checksum-verified, quarantine-stripped on explicit opt-in;
  `WORKTREES_INSTALL_APP=1` / `--with-app` for non-interactive). `make
  install-app` builds + installs from a clone.
- Persistent app log (`~/Library/Logs/net.casadelvalle.worktrees/app.log`):
  every op result, terminal/updater failures, frontend errors, panics, and a
  startup line with version + resolved PATH. Settings → Logs opens the folder
  or tails it. "Check for updates" now acknowledges its result.

### Fixed
- GUI-launched apps inherited launchd's bare PATH (no homebrew → no tmux):
  every place looked dead in the installed .app. The real PATH is resolved
  from the login shell at startup.
- Nav tree: nesting is now DRAWN — per-level plumb-line rails with a lit
  ancestor trail on selection, the (main) row's dot in the project header's
  dot column, a recessed Dormant band, and a tighter indent (rails carry the
  structure, slugs keep their width).
- Settings → Logs "Open folder" was silently rejected by the capability
  system (opener:default has no open-path); now reveals app.log in Finder.

## [0.2.0] - 2026-07-25

The Rust release: one compiled engine behind both the CLI and a desktop app.

### Added
- `worktrees ls --json` (also `WORKTREES_JSON=1 worktrees ls`): a machine-readable
  snapshot (`schema_version` 1) of every place — the main checkout first, then each
  worktree with live derived state (branch/detached, dirty + file count, ahead/behind
  vs upstream, tmux session up/down, last commit, install command, Claude-session
  presence, and a computed `lifecycle_effective`). The human `ls` table is unchanged.
- `worktrees close <name> [name...]` — end a place's tmux session; the worktree,
  branch, and declared state all stay (the inverse of `open`). Resolves a branch to
  its holder worktree, closes adopted sessions (a pane cwd'd in the worktree under
  another name), and `close main` targets the main checkout — unless a worktree is
  literally named `main` (the directory wins).
- **Desktop app** (Tauri, links the engine in-process): multi-project nav tree with
  lifecycle groups (Pinned/Active/Idle + a Dormant fold), embedded tmux terminals
  (attach-not-own), create/switch/close/remove and lifecycle/pin/note from the UI,
  right-click context menus (Enter, open fresh, close session, copy attach command,
  new worktree off a branch, open on GitHub, reveal in Finder, open in editor…),
  a collapsible rail-only nav (⌘B), persisted Settings (UI font scale, terminal font,
  density, nav width, editor command), auto-resume of an existing Claude conversation
  on open, live refresh, and an in-app update check (Settings → Version) that can
  update the installed CLI via the pinned-tag installer.
- Declared lifecycle store (`.worktrees.places.json`, schema-versioned plain JSON):
  saved/archived/abandoned/closed + pin + note, reconciled with live tmux state.

### Changed
- The CLI is now a compiled Rust binary (`crates/worktrees-cli`), behavior-identical
  to the original bash version (gated by the same bats suite — now 137 cases — plus
  real-tmux smokes). `install.sh` fetches a prebuilt binary per platform (macOS/Linux,
  x86_64/arm64) or builds from source with `cargo`; `make install` compiles +
  symlinks the release binary; `bin/worktrees` is a shim that runs the built binary
  from a clone. The legacy bash implementation was retired at full parity.
- Snapshot reads are parallel (bounded fan-out per place and per project) — big
  monorepos with many worktrees list in ~max(latency) instead of sum.
- tmux kill targeting is exact-match only (`-t =name`); the prefix-match fallback
  that could hit a sibling session (`api` → `api-fix`) is gone.

### Fixed
- Claude project-dir detection now mangles every non-alphanumeric character
  (matching Claude Code), so resume detection works for paths with `_` etc.

## [0.1.0] - 2026-07-12

### Added
- Initial release: `new`/`co`, `switch`, `open`, `ls`, `rm` — git-worktree-per-branch
  workflow with a tmux session per worktree (pane 0 AI CLI, pane 1 dependency install + shell).
- Configurable AI pane: `--ai` flag, `$WORKTREES_AI_CMD` (deprecated alias
  `$WORKTREES_CLAUDE_CMD`), `ai_cmd` in `~/.config/worktrees/config`; default `claude`,
  `none` for a plain shell. Resume arg configurable (`$WORKTREES_AI_RESUME_ARG` /
  `ai_resume_arg`, default `-r`).
- Namespace prefix: `$WORKTREES_PREFIX` > `.worktree-prefix` file > user config > repo dir name.
- Runs on stock macOS bash 3.2 and Linux; git ≥ 2.23; tmux optional (≥ 1.9) — `new`
  degrades to `--no-tmux`, `open` requires it.
- `install.sh` curl installer (release-pinned, checksum-verified, `~/.local/bin`) and
  `make install` (symlink) for clones.

### Provenance
Extracted from the Casa del Valle monorepo's `scripts/worktrees.sh`, minus its
docker/stack-mode and AI-question features.
