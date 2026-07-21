# Changelog

All notable changes to this project are documented here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/).

## [Unreleased]

### Added
- `worktrees ls --json` (also `WORKTREES_JSON=1 worktrees ls`): a machine-readable
  snapshot (`schema_version` 1) of every place — the main checkout first, then each
  worktree with live derived state (branch/detached, dirty + file count, ahead/behind
  vs upstream, tmux session up/down, last commit, install command, Claude-session
  presence, and a computed `lifecycle_effective`). The human `ls` table is unchanged.
  Pure-bash JSON serialization (RFC 8259-safe, control-char escaping) — no `jq` needed.

### Changed
- The CLI is now a compiled Rust binary (`crates/worktrees-cli`), behavior-identical
  to the bash version (gated by the same bats suite). `install.sh` fetches a prebuilt
  binary per platform (macOS/Linux, x86_64/arm64) or builds from source with `cargo`;
  `make install` compiles + symlinks the release binary. The legacy bash engine is kept
  at `bin/worktrees.bash` as a parallel test gate; `bin/worktrees` is a shim that runs
  the built binary from a clone.

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
