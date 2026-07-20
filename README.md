# worktrees

One git worktree per branch, one tmux session per worktree — pane 0 runs your AI
CLI (claude, codex, opencode, …), pane 1 installs deps and gives you a shell.

**A worktree is a PLACE** (directory, tmux session + AI CLI history,
node_modules); **a branch is a unit of work that flows through it**. The place
is named after its first branch unless `--name` says otherwise; `switch` moves
it to the next branch without touching anything expensive. Ship a branch, switch
the place to the next one, keep working.

```
worktrees new feat/checkout          # worktree + branch + tmux (AI | deps+shell)
worktrees ls                         # what places exist, what's on them
worktrees switch feat/checkout-v2    # same place, next branch (run inside it)
worktrees open feat-checkout         # reattach later
worktrees rm feat-checkout           # tear the place down when it's done
```

Works in **any** git repo. No config required.

## Install

Fresh machine (installs the latest release to `~/.local/bin`):

```sh
curl -fsSL https://raw.githubusercontent.com/penard-monkey/worktrees/main/install.sh | bash
```

Teams should pin the tag for reproducibility:

```sh
curl -fsSL https://raw.githubusercontent.com/penard-monkey/worktrees/v0.1.0/install.sh | bash
```

Or clone (the clone is the dev loop — `git pull` upgrades in place):

```sh
git clone https://github.com/penard-monkey/worktrees && cd worktrees && make install
```

Re-running the installer upgrades. `install.sh --uninstall` removes the binary
(your repos' worktrees and tmux sessions are untouched).

**Requires:** git ≥ 2.23. tmux ≥ 1.9 recommended (`new` degrades to `--no-tmux`
without it; `open` needs it). Runs on stock macOS bash 3.2 and Linux.

## Commands

```
worktrees new <branch> [base]         create a worktree + tmux (AI | shell)
worktrees new <branch> --name <topic> ...place named independently of the branch
worktrees co  <branch>                checkout a REMOTE branch (fetch if needed)
worktrees switch [<worktree>] <branch> [base]   move a worktree to another branch
worktrees open <name>                 reopen a worktree's tmux session
worktrees ls [--json]                 list worktrees + their state (--json: machine-readable)
worktrees rm <name> [name...]         tear one (or more) down
worktrees                             (no args) → ls
```

`new`/`co`/`switch` are do-what-I-mean: reuse an existing worktree, check out an
existing local **or** remote branch (fetching it first), or create a new branch
off `[base]` (default `main`). `origin/feat/x` is accepted and normalized to
`feat/x`. If a branch already lives in a differently-named worktree (after a
`switch`), `new`/`open` find and reuse that place instead of failing.

Flags:

| Command | Flags |
|---|---|
| `new`/`co`/`open` | `-r/--resume` (append the AI resume flag) · `--ai <cmd>` (AI pane command for this run) |
| `new`/`co` | `--no-install` · `--no-tmux` · `--no-attach` · `--no-fetch` · `--name <topic>` |
| `switch` | `--force` (despite uncommitted changes) · `--no-fetch` · `-y` |
| `rm` | `--branch` (delete the branch too) · `--force` · `-y/--yes` |

Guards you'll be glad exist: dirty worktrees refuse to `switch`/`rm` (override
with `--force`); a stale *unregistered* dir under `.worktrees/` is never treated
as a worktree (git would silently operate on your main checkout); `switch` from
inside worktree A targeting worktree B asks first; a typo'd worktree name can't
silently mint a junk branch.

## The tmux layout

Each worktree gets a session named `<prefix>-<slug>`: pane 0 launches your AI
CLI through an interactive shell (so shell aliases resolve), pane 1 runs the
detected package-manager install (pnpm/bun/yarn/npm, by lockfile) and drops to a
shell. Sessions are reused, never duplicated — `open` finds a session already
living in the worktree even under a different name.

## JSON output

`worktrees ls --json` (or `WORKTREES_JSON=1 worktrees ls`) emits a machine-readable
snapshot instead of the table — for editors, scripts, and tooling. The human `ls`
output is byte-for-byte unchanged. Shape (`schema_version` 1):

- a wrapper `{schema_version, repo, prefix, places_file, places:[…]}`;
- the **main checkout first** (`slug:"(main)", is_main:true`), then each worktree in
  the same recency order as the table;
- per place: `slug, path, branch` (null when detached, with `detached:true`),
  `dirty, dirty_files, ahead, behind, upstream` (the last three null when there's no
  upstream), `created, created_epoch, last_commit_epoch, last_commit_subject,
  tmux_session:{name,up}, claude_session_present, install_cmd`, and `lifecycle_effective`.

All state is derived live on every call — nothing is cached. `stack` and `declared`
are reserved (null for now). No `jq` required to produce it.

## Configuration

Precedence: **flag > environment > user config > default.** User config lives at
`~/.config/worktrees/config` (respects `$XDG_CONFIG_HOME`) — `key = value`
lines, `#` comments. It is parsed as data, never executed.

| What | Flag | Env | Config key | Default |
|---|---|---|---|---|
| AI pane command | `--ai <cmd>` | `WORKTREES_AI_CMD` | `ai_cmd` | `claude` |
| AI resume flag (`-r` appends it) | — | `WORKTREES_AI_RESUME_ARG` | `ai_resume_arg` | `-r` |
| Session/name prefix | — | `WORKTREES_PREFIX` | `prefix` | repo dir name |

```ini
# ~/.config/worktrees/config
ai_cmd = codex
ai_resume_arg = resume
```

- `ai_cmd = none` (or `--ai none`) → pane 0 is a plain shell, no AI.
- `WORKTREES_CLAUDE_CMD` is honored as a **deprecated** alias of `WORKTREES_AI_CMD`.
- A repo can pin its prefix with a committed `.worktree-prefix` file (one line);
  the env var wins over it.
- Pane 0 hands the command to your `$SHELL -ic` — aliases work; assumes a
  POSIX-ish (bash/zsh/sh) login shell.

Examples: `--ai claude`, `--ai "claude --model opus"`, `--ai codex`,
`--ai opencode`, `--ai none`.

## Compatibility notes

- macOS: stock `/bin/bash` 3.2 is fully supported (CI runs the whole suite on it).
- `.worktrees/` is added to `.git/info/exclude` automatically — worktrees never
  show up as untracked files.
- `rm` deletes the worktree and its tmux session; the **branch survives** unless
  you pass `--branch`.

## Development

```sh
git clone --recurse-submodules https://github.com/penard-monkey/worktrees
make check            # shellcheck + bash-3.2 gates + bats suite
make test-real-tmux   # 3 integration smokes against real tmux
```

Tests are bats-core (vendored as submodules); the suite fakes tmux with a PATH
shim so every pane command is assertable, and CI runs ubuntu + macos, the latter
twice — once under stock bash 3.2.

## License

MIT
