# Codex folder-access prompt, 0.160.1

Two captures, both visible-screen `capture-pane -p` with no keys sent:

- `subdirectory.txt` — below.
- `linked-worktree-80col.txt` — captured 2026-10-28 on macOS, from the same
  installed `codex-cli 0.160.1`, launched by worktrees itself
  (`worktrees open live-a --ai codex --force`, so with every flag a lane gets:
  `--no-alt-screen`, `--approve-for-me`, the `-c` overrides) in a linked
  worktree of a new, untrusted scratch repo, on a private tmux server
  (`TMUX_TMPDIR`, `TMUX` unset), 80×24. Both paths wrap mid-word at this
  width. The private server was killed by its socket afterwards. The user's
  `config.toml` hashed the same before and after, Codex wrote no rollout and
  no trust entry, and it touched only its own runtime files (sqlite WALs,
  logs, `models_cache.json`, `shell_snapshots`).

## subdirectory.txt

Captured 2026-10-24 on macOS from the installed `codex-cli 0.160.1`
(VS Code extension `openai.chatgpt-26.930.61225-darwin-arm64`), in a new,
untrusted git repository's `subdir`. Private server `tmux -L wt-trust-01601`,
120 columns × 36 rows; visible-screen `capture-pane -p`.

Command: `codex --no-alt-screen --no-daemon`. The latter isolates this probe
from the user's persistent app-server. No prompt was submitted and **no keys
were sent**. The private server was killed by its explicit name afterwards.
SHA-256 of the user's `config.toml` was identical before and after.

The normal sandbox blocked runtime startup, so the capture ran with shell
sandbox escalation. Codex touched runtime caches/logs; concurrent Codex lanes
also wrote state during this interval, so those shared files were not removed.
No trust entry was created. The scratch repo remained under
`/private/tmp/wt-trust-investigation/repo` for provenance checks.

The last line, `enter continue · esc quit`, anchors this modal. The question
in history alone must never mark an idle composer as waiting. Trust is a
host-only permission, not a question with freely answerable choices.

Attached to its daemon, Codex ends the same prompt on `enter continue · esc
back` instead (upstream's `trust_directory` snapshots; not captured here).

Trust lookup was checked against upstream tag `rust-v0.160.1`. Whether the
prompt appears is `codex-rs/tui/src/config_update.rs::read_remote_project_trust`,
asked by `codex-rs/tui/src/onboarding/directory_trust.rs`, with the loader's
`decision_for_dir` (`codex-rs/config/src/loader/mod.rs`) agreeing and
`codex-rs/git-utils/src/trust.rs` resolving the main repository root. NOT
`ConfigToml::get_active_project_for_lookup` (`config/src/project_trust.rs`),
which lets a cwd entry with no `trust_level` win: that picks the active
project, not whether to ask.
