# Session: tmux install gate + missing-tmux banner

- **Date:** 2026-07-30
- **Worktree:** bug-fixes
- **Branch(es):** bug-fixes → PR [#65](https://github.com/penard-monkey/worktrees/pull/65) (squash-merged as `0134695`)
- **Release tag:** none (rides in next release's `[Unreleased]`)
- **Planning files:** `planning.tar.gz` in this dir (task_plan / findings / progress)

## What shipped

- **install.sh** — tmux is now a hard requirement (`require_tmux()` helper).
  macOS: brew + tty → `[Y/n]` offer that runs `brew install tmux` and
  re-verifies; brew missing → error pointing at https://brew.sh; no tty →
  error with the command. Linux: detects apt-get/dnf/pacman/zypper, prints
  the exact command, exits 1 without running anything. Header documents the
  requirement. CLI runtime degrade (`new` → `--no-tmux`) untouched.
- **app/src-tauri/src/lib.rs** — `fixup_gui_path()` always appends
  `~/.local/bin:~/bin:/opt/homebrew/bin:/usr/local/bin` after the
  shell-derived PATH (was: only when the login-shell probe failed). New
  async `tmux_check(refresh)` command: `refresh=true` re-runs
  `fixup_gui_path()` then probes `have_tmux()`, logging when tmux appears.
- **app/src/App.tsx + App.css** — module-scope `TmuxBanner` above the top
  bar when tmux is missing (amber, tokens-styled): install hint + Re-check
  button. Success clears the banner and refreshes places; failure shows
  "still not found". Errors via `fail()`.
- **app/src/mock/install.ts** — stateful `tmux_check`; `?notmux` (Re-check
  finds tmux, banner retires) and `?notmux=stuck` (never found) fixtures.
- **CHANGELOG.md** — `[Unreleased]` Added/Changed/Fixed entries.

## Decisions

- **tmux mandatory at install, NO escape hatch** (David, over the offered
  `WORKTREES_INSTALL_NO_TMUX=1`). A place IS a tmux session; installing
  without tmux produced a half-working tool.
- **macOS offers to run `brew install tmux`** rather than hard-erroring or
  keeping the old warn — reuses the /dev/tty prompt pattern proven by the
  desktop-app install prompt. Brew absent → error, never auto-install brew.
- **Linux never runs a package manager** — names the exact command, exits.
- **App fix = banner + Re-check button** (over restart-required note or
  auto re-probe on a timer): the login-shell probe can cost ~5s, so
  re-detection is user-triggered, not polled.
- **`fixup_gui_path` always appends std dirs** — a profile that never runs
  `brew shellenv` reported a PATH without /opt/homebrew/bin, hiding a
  brew-installed tmux from the GUI app even across restarts.

## Dead ends / gotchas

- **"No way to register tmux after the fact" was half-myth.** The engine
  (`have_tmux()`) probes live and never caches; a place created without
  tmux needs zero migration — first Enter/`open` creates its session. The
  real blockers were (1) GUI PATH resolved exactly once at startup, (2)
  the shellenv-less-profile PATH hole, (3) zero UI signal (everything just
  looked dead). Fixing "registration" meant fixing PATH + visibility, not
  the engine.
- **Worktree env, not repo:** bats submodules were uninitialized
  (`git submodule update --init --recursive`) and app/node_modules absent —
  `pnpm install` requires Node ≥ 22.13 (shell default 22.12 is rejected by
  pnpm 11). Cost the implement agent its first test runs.
- `std::env::set_var` at runtime (Re-check) is technically racy against
  concurrent `getenv` on POSIX — accepted as matching the existing
  startup-time pattern; see Follow-ups.

## Verification

- All gates green (implement agent + independent re-run of lint/tsc):
  release CLI build, `make test` 238 ok, `make lint` (shellcheck + bash-3.2
  gate), `cargo test -p worktrees-core` 137 ok, app `tsc --noEmit` +
  `cargo check -p app`. CI green on both OSes before merge.
- Mock harness driven headlessly (Playwright, vite): `?notmux` banner
  renders and retires on Re-check; `?notmux=stuck` shows "still not found";
  default URL no banner, no console errors.
- `require_tmux` branches smoke-tested with fake bins: Linux-dnf,
  Linux-unknown, macOS-no-brew, macOS-brew-no-tty.

## Follow-ups

- **Manual:** interactive macOS `[Y/n]` brew prompt never machine-verified
  (needs a real controlling tty) — one live run on a tmux-less mac.
- **PATH hygiene:** each Re-check prepends the std-dirs block again
  (dups harmless but unbounded); a dedupe pass in `fixup_gui_path()` is the
  tidy fix. Same visit could reconsider runtime `set_var` vs a computed
  PATH handed to spawns.
