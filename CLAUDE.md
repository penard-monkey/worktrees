# worktrees — working notes for Claude

One git worktree per branch, one tmux session per worktree. A worktree is a
durable PLACE; a branch is work that flows through it. Full design docs:
DESIGN.md (app), MIGRATION.md (bash→Rust history).

## Architecture

- **One engine**: `crates/worktrees-core`. The CLI (`crates/worktrees-cli`,
  binary `worktrees`) and the Tauri app (`app/src-tauri`) BOTH use it — the app
  links it in-process (no subprocess). The legacy bash engine is retired.
- git/tmux are **shelled out** on purpose (faithful port; keeps the bats
  fake-shim harness intercepting the compiled binary). Don't switch to libs.
- State split: **derived** (live git/tmux, recomputed) vs **declared**
  (`.worktrees.places.json` — lifecycle/pin/note, plain JSON, no DB).
  Terminals ATTACH to tmux, never own shells. Session name =
  `<prefix>-<slug>` with `.` → `-`.
- One version source: workspace `Cargo.toml`. The app crate + tauri.conf
  inherit it; `test/misc.bats` asserts the binary against it.

## Gates (run before any PR)

```sh
cargo build --release -p worktrees-cli   # FIRST — bin/worktrees shim prefers
                                         # target/release; a stale release binary
                                         # makes bats fail mysteriously
make test           # bats suite vs the Rust binary (fake git/tmux shims)
make lint           # shellcheck + bash-3.2 gate (shim + install.sh)
cargo test -p worktrees-core
cargo test -p worktrees-cli   # MCP protocol unit tests live here
cargo test -p app --lib      # `check`/`build` do NOT compile `mod tests`
cd app && ./node_modules/.bin/tsc --noEmit && cargo check -p app
```

CI mirrors these + builds the app crate on both OSes. Squash-merge PRs.

**A stale release binary does not only make bats FAIL — it can make it PASS.**
The note above says "fail mysteriously", which is the friendlier half. Edit a
crate *after* the release build and `make test` happily green-lights the OLD
binary; the same goes for the `ls --json` diff below, which then compares the
shipped binary against another copy of itself. Both read `target/release`, and
neither notices it predates the change. Rebuild, then confirm before believing a
result: `[ target/release/worktrees -nt crates/worktrees-core/src/store.rs ]`.

**Report cargo with a grep, not a tail.** `cargo test -p worktrees-core | tail -4`
prints `running 0 tests / ok. 0 passed` — that is the **Doc-tests** block, and
the real `running 207 tests` is above the cut. A tail-truncated run reads exactly
like a crate with no tests. Use `grep -E "^running|^test result"`. Related shell
trap when scripting gates: `grep -c` **exits non-zero on a count of 0**, so
`... | grep -c "^not ok" && next-gate` silently skips everything after it — a
gate that never ran looks identical to a gate with no output.

**A pipeline's exit status is the LAST stage's.** `make test | tail -15` exits 0
because `tail` did, and a mid-stream `not ok` scrolls off a 15-line window — so
the run reads green whatever bats said. Redirect, then check both halves:
`make test > log 2>&1; echo $?` plus `grep -cE '^not ok' log`. Same family as the
grep traps above, and the reason to run gates with `make -C <repo-root>`: the
Bash tool's cwd persists between calls, so an earlier `cd app` turns a later
`make test` into "No rule to make target `test`" — a failure that looks like the
change broke the build.

**A new test must be shown to FAIL first.** ROADMAP's zombie-children item
records a regression test that passed identically with and without its fix.
Break the thing under test (drop the `skip_serializing_if`, restore the old
line), watch it go red, then restore. Two tests this repo now relies on were
confirmed this way.

**Consolidating a git invocation? Diff `ls --json` against the SHIPPED binary.**
Folding three git calls into one `status --porcelain=v2` silently changed
`upstream` for one worktree — v2 reports the CONFIGURED upstream, `rev-parse
@{u}` reported only one that RESOLVES. Neither bats nor the 205 unit tests
covered it; only the output diff did. `~/.local/bin/worktrees` is the last
release, so it is the reference.

**Counting subprocesses: shim the CLI, never the app.** Wrap `git`/`tmux`/
`stat`/`date` in counting wrappers on PATH and run `worktrees ls --json` — the
identical `snapshot()` path (`spawn-count.sh` in the worktree's cache dir).
`ps`-sampling undercounts by ~3× and misses sub-millisecond spawns entirely. The
shim can't measure the APP: `fixup_gui_path()` prepends the login-shell PATH at
startup and keeps the inherited one only as a trailing fallback, so the real git
wins.

A FRESH worktree needs two bootstraps first, and both fail confusingly:
`git submodule update --init --recursive` (without it `make test` dies with a
bare "No such file or directory" naming the bats binary, not the submodule),
and `pnpm install` in `app/` under Node >= 22.13 (`nvm use 22.23.2`).

**AI profiles have a manual gate too.** Everything claude-side (does the config
swap apply, does session adoption still see `claude`, does auto-resume resume)
is invisible to the bats suite — there is no fake claude. Re-run
`docs/ai-profiles-manual-checks.md` whenever the `claude` binary is upgraded.

## Tauri app — hard-won rules

- **Commands must be `async fn`** — sync handlers run on the main thread and
  freeze the UI for every git/tmux shell-out.
- **GUI launches get launchd's bare PATH** (no homebrew → no tmux).
  `fixup_gui_path()` in lib.rs resolves the login-shell PATH at startup —
  don't add subprocess calls that assume PATH before it runs.
- **Components defined inside App() remount every render** (new identity) —
  anything with local state or input focus goes at module scope with props.
- The mock harness (`pnpm dev:mock`, `app/src/mock/install.ts`) must track
  every command in lib.rs — it's how the UI is developed/driven headlessly
  (Playwright). Port 1420 = `tauri dev`; run the harness on another port.
- **The mock answers INSTANTLY, and that hides a whole class of bug.** Its
  invokes resolve in a microtask, so two `list_workspace` sweeps never overlap
  and there is no gap between "write done" and "refresh returned" — the real one
  is a git fan-out over every project (0.28s for one project with nine
  worktrees, seconds across a workspace). Three v0.12.x bugs passed gates,
  review and harness checks and were then found by running the real app; all
  three lived in timing the harness cannot express. Before releasing anything
  touching refresh, optimistic UI or per-place state, run it for real:
  `app/scripts/sandbox.sh --app` (isolated identifier + tmux prefix, so it
  cannot collide with your installed app — and NOTE bare `sandbox.sh` is the
  CLI sandbox meant to be `eval`'d, it does not launch the app). Two tools now
  cover the shapes already hit: `?slowlist=<ms>` makes the mock's
  `list_workspace` slow, and `app/scripts/race-check.mjs` drives the real
  `refresh`/`commitWs`/`patchDeclared`/`mutate` source under controlled
  promise-resolution orders (`node app/scripts/race-check.mjs [App.tsx]`, exits
  non-zero on failure — it fails on v0.12.0, which is how it earns trust).
- **HMR is dead inside `.worktrees/`** — chokidar ignores dot-directories, so
  vite never sees the edit and keeps serving the PRE-edit file. A reload and a
  `touch` both "work" and change nothing; a real fix looks like it failed.
  Restart with `--force` after every source edit, and when a change seems not to
  apply, diff what the server serves (`curl -s localhost:PORT/src/App.css`)
  against disk before debugging the change itself.
  **Killing the harness needs a CONTENT check, not a port check.** Two vite
  instances can hold the same port — kill one and `lsof -ti:PORT` still answers,
  so "port free" reads as true while a survivor serves the PRE-edit file and a
  test "verifies" the old code. Use `lsof -ti:PORT -sTCP:LISTEN` (plain `-ti`
  also returns Chrome's network-service helpers), then grep the served file for
  something the edit added.
- Assert layout in the harness (`getComputedStyle`), don't eyeball it — a CSS
  rule killed by a stray `*/` still renders a plausible-looking widget.
- **portable-pty's `Child::kill()` sends SIGHUP, not SIGKILL** (crate
  `lib.rs:347`), and an interactive `/bin/sh` on a pty whose master is still
  open SURVIVES it. The app only gets away with this because dropping the
  `Shell` closes the master and the EOF finishes the job. Two more traps in the
  same family, each of which cost a >10-minute hang: **a pty test must DRAIN the
  master** (without a reader the shell fills the pty buffer and the child wedges
  mid-exit — `ps` state `E`, never reaped, so even SIGKILL + `wait()` blocks
  forever), and **`process_id()` keeps returning a REAPED pid** (portable-pty's
  impl is an unconditional `Some(self.id())`, and `list_shell_sessions` reaps on
  every dock mount — so anything sampling by pid needs a `try_wait` liveness
  check first, or it eventually reads a stranger's process).
- **Playwright: a two-click arm needs BOTH clicks in one `browser_evaluate`.**
  The arm expires in 4s — longer than one MCP round-trip — and the button's
  `title` CHANGES when armed, so selecting on the unarmed title silently hits a
  DIFFERENT row. A run that did nothing at all reads exactly like a run that
  passed. This is the one exception to one-click-per-evaluate above, which
  exists for reading state, not for timing; read state in the NEXT call.
- **One click per `browser_evaluate`.** React batches, so several `.click()`s in
  a single eval return before any of them render — the DOM you read back is the
  one from before the clicks, which reads as "the tree ignored them". Drive
  state changes one call at a time and query in the next.
- Plugin permissions live in `app/src-tauri/capabilities/default.json`;
  `opener:default` has open-url + reveal-item-in-dir but NOT open-path —
  a missing permission rejects the invoke silently. Never swallow errors:
  route failures through `fail()` (frontend) / `applog` (backend).
- App log: `~/Library/Logs/net.casadelvalle.worktrees/app.log` (Settings →
  Logs). Persisted UI settings: `ui-state.json` in the app config dir — written
  WHOLE-BLOB by the frontend, so the backend must never write into it (its own
  update would be erased by the next settings save; that is why each dock shell
  tab's last directory lives in a separate backend-owned `shell-cwds.json`).
- Design tokens: `app/src/tokens.css` — everything scales off `--ui-rem`;
  terminal font is independent (`--term-*`). No UI libraries, plain CSS. "No UI
  libraries" means no COMPONENT/design-system libraries and no editor — a pure
  PARSER that emits data we render ourselves is allowed, and `marked` (lexer
  only, for the dock's markdown) is the one instance. Syntax highlighting is
  hand-rolled in `app/src/highlight.ts` for the same reason.
- **Never run the bundle's binary to probe it.**
  `target/release/bundle/macos/worktrees.app/Contents/MacOS/app --version` is the
  GUI entry point — it LAUNCHES a second instance instead of printing a version.
- **`document.visibilityState` works here** — WKWebView fires `visibilitychange`
  on minimize, ⌘H, Space switch and full occlusion (confirmed on a real build via
  logged transitions). The Tauri issues claiming otherwise are Windows/WebView2.
  No `objc2`/NSWindow occlusion observer needed. It does NOT fire on plain focus
  loss, which is correct: a visible-but-unfocused window is still being read.
- Measuring anything in the app: `app.log` timestamps are **UTC**, most harness
  output is local. Cross-reference before trusting a window-state measurement —
  a "hidden" run that showed MORE work turned out to have flapped visible five
  times mid-window.
- macOS FS is case-insensitive: `Settings.tsx` collided with `settings.ts`
  once (component is `SettingsSheet.tsx`). Watch new filenames.

## Release

1. CHANGELOG: move `[Unreleased]` into `## [x.y.z] - date` (release.yml uses
   the section as notes; the app shows it as "What's new" — it ships in the
   binary via include_str!).
2. Bump workspace `Cargo.toml` → PR → merge.
3. `make release VERSION=x.y.z` → `git push origin main vx.y.z`.
4. release.yml: CLI ×4 targets + SIGNED app bundles ×2 + latest.json.
   Updater signing key: repo secret `TAURI_SIGNING_PRIVATE_KEY`; local backup
   `~/.tauri/worktrees-updater.key` — irreplaceable, never commit it.
5. Users update from inside the app (Settings → Version: CLI + app buttons)
   or by re-running install.sh.

## Local installs

- CLI stable: `install.sh` (copies). `make install` SYMLINKS the clone's
  build — every rebuild silently becomes "stable"; don't use it for that.
- App: `make install-app` → /Applications (local builds skip Gatekeeper).

## Decisions

`docs/adr/` holds decisions that must survive being forgotten. Read them before
adding config surface. **ADR 0001: a cloned repo never supplies argv** — no
`[hooks]`, no `[infra] up/stop/down`, no per-place `up_cmd`. `projcfg.rs`'s
`USER_ONLY_KEYS` makes them hard parse errors, and `DESIGN.md` still *describes*
them (marked superseded) because it was written before the reversal.

## Planning docs

`task_plan.md` / `findings.md` / `progress.md` are gitignored working memory —
read them at session start, keep them current. At close-out they get
tarballed into the session archive (see below). `_tmp/` is a user symlink
(iCloud) where screenshots for review land.

## Scratch files

Screenshots, harness output, and other throwaway artifacts go in
`~/.cache/worktrees/<project>/<worktree-name>/` (e.g.
`~/.cache/worktrees/worktrees/ui-changes/`) — never the repo root.

The Playwright MCP tools can't honour that directly: they refuse any path
outside the repo ("outside allowed roots") and drop their own output in
`.playwright-mcp/`. Let them write into the repo, then MOVE the artifacts to
the cache dir before close-out.

## Close-out ritual

When a work stream is done and the session is about to be `/clear`ed, run
the `/close-out` skill — GLOBAL since 2026-08-10, source in
`~/workspace/claude-skills` (symlinked into `~/.claude/skills`). This repo's
paths, gates, index and branch naming live in `.claude/close-out.md`, which
the skill reads; edit that file, not the skill. Short version: scratch →
`~/.cache/worktrees/…`, session summary + planning tarball →
`docs/sessions/<date>-<slug>/` + a row in `docs/sessions/index.md`
(committed), stragglers → `ROADMAP.md`, one squash-merged PR, then a fresh
branch off origin/main.

**Tag the release from the worktree that already owns `main`** (the repo root).
`git checkout -B main` inside a side worktree moves the SHARED branch ref out
from under it, leaving the root on the new commit with a stale working tree and
phantom "modifications" — the inverse of the release, staged. Recoverable with
`reset --hard`, but check for untracked files first.

**No `checkout -B` is required to hit this.** Any branch checked out in TWO
worktrees does it: whoever moves the ref wins, the other tree keeps a stale
working copy, and its index reads as the inverse of everything that landed in
between — 600 lines of deletions that are not real. The reflog will not show it
(it records only that tree's own checkouts), so prove it before resetting:
`diff <(git diff --cached) <(git diff <branch-tip> <the-commit-you-were-on>)`
empty ⇒ the tree is exactly the old commit and there is nothing local to lose.
Give every worktree its own idle base (`<tree>-next`); `.claude/close-out.md`
lists them.

Branch off a FRESHLY FETCHED `origin/main`, and check with
`git rev-list --left-right --count origin/main...HEAD` — an idle worktree's
last commit can look like the tip and not be. PR numbers are not merge order:
a long-lived PR merges after higher-numbered ones, so a worktree parked on
"close out #83" was a commit behind because #72 landed later.
