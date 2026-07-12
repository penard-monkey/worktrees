# Changelog

All notable changes to this project are documented here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/).

## [Unreleased]

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
