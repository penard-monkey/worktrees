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
cd app && ./node_modules/.bin/tsc --noEmit && cargo check -p app
```

CI mirrors these + builds the app crate on both OSes. Squash-merge PRs.

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
- Plugin permissions live in `app/src-tauri/capabilities/default.json`;
  `opener:default` has open-url + reveal-item-in-dir but NOT open-path —
  a missing permission rejects the invoke silently. Never swallow errors:
  route failures through `fail()` (frontend) / `applog` (backend).
- App log: `~/Library/Logs/net.casadelvalle.worktrees/app.log` (Settings →
  Logs). Persisted UI settings: `ui-state.json` in the app config dir.
- Design tokens: `app/src/tokens.css` — everything scales off `--ui-rem`;
  terminal font is independent (`--term-*`). No UI libraries, plain CSS.
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

## Planning docs

`task_plan.md` / `findings.md` / `progress.md` are gitignored working memory —
read them at session start, keep them current. At close-out they get
tarballed into the session archive (see below). `_tmp/` is a user symlink
(iCloud) where screenshots for review land.

## Scratch files

Screenshots, harness output, and other throwaway artifacts go in
`~/.cache/worktrees/<project>/<worktree-name>/` (e.g.
`~/.cache/worktrees/worktrees/ui-changes/`) — never the repo root.

## Close-out ritual

When a work stream is done and the session is about to be `/clear`ed, run
the `/close-out` skill (`.claude/skills/close-out/SKILL.md`). Short version:
scratch → `~/.cache/worktrees/…`, session summary + planning tarball →
`docs/sessions/<date>-<slug>/` (committed), stragglers → `ROADMAP.md`,
one squash-merged PR, then a fresh branch off origin/main.
