# worktrees

**A durable place for every work stream.** Not a throwaway worktree per branch —
a place you keep: `ui-changes`, `prod-reviews`, `mcp-server`. Each place is a
git worktree + a tmux session — pane 0 runs your AI CLI (claude, codex,
opencode, …), pane 1 installs deps and gives you a shell — and it lives as long
as the work does: an afternoon, or weeks of async iteration until it's right.

**Branches flow through places.** The place keeps the expensive parts
(directory, AI CLI history, node_modules, the running session); a branch is the
unit of work currently on it. Ship a branch, `switch` the place to the next one,
keep working. The place is named after its first branch unless `--name` says
otherwise; `switch` moves it to the next branch without touching anything
expensive.

```
worktrees new feat/checkout          # worktree + branch + tmux (AI | deps+shell)
worktrees ls                         # what places exist, what's on them
worktrees switch feat/checkout-v2    # same place, next branch (run inside it)
worktrees open feat-checkout         # reattach later
worktrees rm feat-checkout           # tear the place down when it's done
```

Works in **any** git repo. No config required.

## Desktop app

Same engine, a native window. The macOS app (Tauri) links the identical Rust
core in-process — no daemon, no subprocess — so everything the CLI knows, the
app shows live: every place grouped by how active it is, ahead/behind and dirty
state per branch, and a tmux-attached terminal for each session right in the
window. Create a place, drop into its session, switch branches, tear it down —
without leaving the app.

<p align="center">
  <img src="docs/media/desktop-flow.gif" width="820" alt="Creating a place and opening its session in the worktrees desktop app">
</p>

<p align="center"><sub><i>New → list → open — driven headlessly against the built-in mock harness. The real app embeds a live tmux terminal running your AI CLI.</i></sub></p>

Every place, grouped by how active it is, with a home panel to resume where you left off:

<p align="center">
  <img src="docs/media/desktop-overview.png" width="820" alt="worktrees desktop app — workspace overview">
</p>

Open a place to see its branch state, notes, and embedded terminal:

<p align="center">
  <img src="docs/media/desktop-session.png" width="820" alt="worktrees desktop app — a place with its embedded terminal">
</p>

Install it alongside the CLI on macOS (`WORKTREES_INSTALL_APP=1`, or answer the
installer prompt; from a clone, `make install-app`) — see [Install](#install).

## Install

Fresh machine (installs the latest release to `~/.local/bin`):

```sh
curl -fsSL https://raw.githubusercontent.com/penard-monkey/worktrees/main/install.sh | bash
```

Teams should pin the tag for reproducibility:

```sh
curl -fsSL https://raw.githubusercontent.com/penard-monkey/worktrees/v0.1.0/install.sh | bash
```

The installer fetches the prebuilt binary for your platform (macOS/Linux,
x86_64/arm64); with no match, or `WORKTREES_INSTALL_FROM_SOURCE=1`, it builds
from source with `cargo`.

On macOS it also **offers the desktop app** (`worktrees.app` → /Applications,
checksum-verified; the unsigned bundle's quarantine attr is stripped on your
explicit opt-in). Non-interactive runs skip the prompt — opt in/out explicitly:

```sh
WORKTREES_INSTALL_APP=1 \
  curl -fsSL https://raw.githubusercontent.com/penard-monkey/worktrees/main/install.sh | bash
```

From a clone, `make install-app` builds the app locally and installs it to
/Applications (no signing or quarantine involved).

Or clone and build (`make install` compiles the release binary and symlinks it —
`git pull && make install` upgrades):

```sh
git clone https://github.com/penard-monkey/worktrees && cd worktrees && make install
```

Re-running the installer upgrades. `install.sh --uninstall` removes the binary
(your repos' worktrees and tmux sessions are untouched).

## Updating

Re-running the installer **is** the updater — it resolves the latest release,
prints the old → new version, verifies the checksum, and replaces the binary
in place:

```sh
curl -fsSL https://raw.githubusercontent.com/penard-monkey/worktrees/main/install.sh | bash
worktrees --version   # confirm
```

Roll back (or hold) a version by pinning the release tag:

```sh
WORKTREES_INSTALL_VERSION=v0.1.0 \
  curl -fsSL https://raw.githubusercontent.com/penard-monkey/worktrees/main/install.sh | bash
```

The same re-run updates the desktop app when you opt in (`WORKTREES_INSTALL_APP=1`
or answer the prompt); quit + reopen the app to pick up the new version.

From a clone instead: `git pull && make install` (note: `make install` symlinks
the clone's release build — later `cargo build`s in that clone update it too.
For a frozen copy, `install -m 755 target/release/worktrees ~/.local/bin/worktrees`).
App from a clone: `git pull && make install-app`.

Updates never touch your repos' state: worktrees under `.worktrees/`, the
declared store (`.worktrees.places.json`, schema-versioned), tmux sessions, and
`~/.config/worktrees/config` all survive binary swaps. Running sessions keep
running — the CLI attaches to tmux, it doesn't own it.

**Requires:** git ≥ 2.23. tmux ≥ 1.9 recommended (`new` degrades to `--no-tmux`
without it; `open` needs it). Prebuilt binaries for macOS + Linux (x86_64/arm64);
building from source needs a Rust toolchain.

## Commands

```
worktrees new <branch> [base]         create a worktree + tmux (AI | shell)
worktrees new <branch> --name <topic> ...place named independently of the branch
worktrees co  <branch>                checkout a REMOTE branch (fetch if needed)
worktrees switch [<worktree>] <branch> [base]   move a worktree to another branch
worktrees open <name>                 reopen a worktree's tmux session
worktrees close <name> [name...]      end the tmux session (worktree stays; also: main)
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
| `new`/`co`/`open` | `-r/--resume` (append the AI resume flag) · `--ai <cmd>` (AI pane command for this run) · `--no-spare` (single pane — no spare shell, and for `new` no auto-install) |
| `new`/`co` | `--no-install` · `--no-tmux` · `--no-attach` · `--no-fetch` · `--name <topic>` · `--brief <text>` (write the agent's task to `.planning/brief.md` and launch claude on it) |
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

### Running the desktop app from source

The app is Tauri, so `cargo` is driven for you — there is no `cargo run`. Node
must match `.nvmrc` (pnpm 11 refuses anything older), and dependencies are
installed once per clone:

```sh
nvm use                      # or any node >= the version in .nvmrc
pnpm --dir app install       # first time only
make dev-app                 # or: pnpm --dir app tauri dev
```

That builds the `app` crate and serves the frontend on **port 1420**, with hot
reload on the TypeScript/CSS side; a Rust change rebuilds and relaunches the
window. Real git and tmux are used, so it acts on whatever projects you have
registered.

To work on the UI alone — no Rust build, no tmux, fake backend — use the mock
harness, which runs the real `App.tsx` against fixtures in a plain browser:

```sh
pnpm --dir app dev:mock --port 1425    # any port but 1420
```

Both are development loops. To actually *use* a locally built app, see
`make install-app` under [Install](#install) — that produces the bundle and puts
it in `/Applications`.

### Scripts

| Script | What it does | Touches |
|---|---|---|
| `sandbox.sh` | Builds the current branch and hands you an isolated worktrees to test it in. `--app` launches the desktop app against it. | a scratch repo, or one you name with `--repo` |
| `record-readme.sh` | Regenerates the README's desktop media (`docs/media/desktop-*.{gif,png}`). | nothing — mock harness |
| `shoot-profiles.sh` | Regenerates the AI-profiles screenshots for `docs/ai-profiles.html`. | nothing — mock harness |
| `record-profiles.sh` | Regenerates that page's walkthrough clip (`walkthrough.mp4`). | nothing — mock harness |

The three media scripts drive the **mock harness** — the real `App.tsx` against
fixtures, in a plain browser — so they are deterministic and cannot touch your
projects. The `.py` file beside each is the Playwright driver; run the `.sh`,
which starts the harness, waits for it, drives it, encodes, and cleans up.

`sandbox.sh` is the exception: it builds and runs the REAL binary, against a
scratch repo by default and against whatever you pass to `--repo` if you ask.

They need Python Playwright and Chromium once — `pip install playwright &&
playwright install chromium` — plus `ffmpeg` for the two that record video.

#### Testing a branch without disturbing an app you already have open

```sh
eval "$(app/scripts/sandbox.sh)"    # CLI: isolated env + a scratch repo
app/scripts/sandbox.sh --app        # …or the desktop app against that sandbox
app/scripts/sandbox.sh --clean
```

This matters more than it sounds. tmux session names are `<prefix>-<slug>`
derived from the repo, so **a second build computes the same name and attaches to
the session your open app is using** — closing it in one kills it in the other.
The sandbox takes a per-branch prefix (`sbx-<branch>-<slug>`), which both
prevents that and is how you tell them apart:

```sh
tmux ls | grep sbx-      # sandboxes, one prefix per branch
tmux ls | grep -v sbx-   # yours, untouched
```

It also isolates the AI-profile store and, with `--app`, overrides the bundle
identifier so the sandbox app gets its own `ui-state.json` instead of sharing
yours.

## License

MIT
