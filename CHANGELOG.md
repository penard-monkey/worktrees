# Changelog

All notable changes to this project are documented here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/).

## [Unreleased]

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
