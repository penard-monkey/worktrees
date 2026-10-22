---
title: "Rehearsal: updating v0.42.0 to the Server Edition"
---

> Archived from the `rehearse-server-edition-update` lane's cache dir: its
> `report.md` followed by `sidecar-merge.md`, verbatim except that home paths
> are written `~/…` and the throwaway updater key's password is left out. The
> paths it cites (`evidence/`, `bundles/`, `home/`, the scripts) stayed in
> `~/.cache/worktrees/worktrees/rehearse-server-edition-update/` and are not
> committed. It was run against the PR stack before the release; D1–D3 and
> D5–D7 were fixed before v0.43.0 shipped (see [the summary](summary.md)).

# Rehearsal: updating v0.42.0 → the Server Edition (macOS)

Lane `rehearse-server-edition-update`, 2026-10-21, 12:24–15:24 MDT. Coordinator (main);
manual steps done by the user in six relayed batches.

- Base `feat/engine-auto-reconnect` @ 2b3c359 (stack #508 → #516 → #515 → #524 → #525)
- \+ `origin/feat/sidecar-bundle` @ 6e75157 (#513), merge **b161a63** (see `sidecar-merge.md`)
- \+ rehearsal-only bump **d4ce51b** (0.43.0). Local, signed, **nothing pushed**.

Paths are relative to `~/.cache/worktrees/worktrees/rehearse-server-edition-update/`
unless absolute. `$R` = `home/` (the rehearsal HOME). Evidence lives in `evidence/`.

## Verdict in one paragraph

The update itself works. v0.42.0 → 0.43.0 through the real in-app updater:
- relaunched with the env intact;
- started the bundled sidecar as an app-owned serve in under a second;
- left tmux and a live stand-in agent untouched;
- restored the dock tab (cwd, ring, seam, per-tab history);
- migrated host policy;
- serves the same places as the old CLI, and mixes freely with the old CLI and MCP.

Clean quit, crash + adopt, and a later 0.43.0 → 0.43.1 update all behaved as designed: the Exit hook stopped serve with SIGTERM every time, with no Replace and no SIGKILL. **What breaks is the way back and the edges:**
- a downgrade silently resets Codex permissions to the wider default (D6);
- after a crash, a downgraded app runs beside an orphaned serve that overwrites its dock ring (D7) and takes its `worktrees show` requests (D2);
- every MCP setup done from 0.43 would point users' agents at the app-private sidecar (D1).

## Rehearsal-only deviations from a real release (read first)

| What | Why | Where |
|---|---|---|
| `identifier: net.casadelvalle.worktrees.rehearse` | brief: never the real id | `rehearse-{old,new}.conf.json` (`tauri build --config`) |
| `productName` + main window `title` = "Worktrees Server"; the window object is otherwise identical to tauri.conf.json (same in 0.42.0 and 0.43.x). The main executable stays `Contents/MacOS/app` and the sidecar `Contents/MacOS/worktrees`; config/log dirs and serve.json key on the identifier | (main)/user: never confusable with the real app | same |
| updater `endpoints` → `http://127.0.0.1:18743/latest.json`, `dangerousInsecureTransportProtocol: true`, `pubkey` = a throwaway key (`$R/keys`, throwaway password) | the faithful updater path without GitHub or the real key | same |
| **ONE-LINE SOURCE PATCH** in `latest_release_tag()`: `https://github.com/{REPO_SLUG}/releases/latest` → `http://127.0.0.1:18743/releases/latest` (v0.42.0 `app/src-tauri/src/lib.rs:1732`; 0.43.x `crates/worktrees-api/src/ops.rs:763`) | v0.42.0's "Update app → vX" button renders only when `check_update.latest` is set, and that is a hardcoded curl to github.com, which also auto-fires ~5 s after launch. "Install <tag>" (`install_app_version`, hardcoded GitHub) was never used | `rehearsal-patch-0.42.0.diff`, `rehearsal-patch-0.43.x.diff` |
| Every rehearsal process ran under `renv`: `env -i`, `HOME=$R`, `TMUX_TMPDIR=$R/tmux`, `TMPDIR=$R/tmp/`, the real PATH, and **every proxy var → dead `127.0.0.1:9`**, `NO_PROXY=127.0.0.1,localhost` | isolation; nothing reaches github.com (proven: curl → "Failed to connect to 127.0.0.1 port 9"; every app/serve log shows `claude_status: curl exited 7`) | `renv` |
| 0.43.0: CHANGELOG `[Unreleased]` cut to `## [0.43.0]` (as Release step 1 does). 0.43.1: local bump of an archive copy | What's new only shows numbered sections | d4ce51b, `src-0.43.1` |
| Bundles built from `git archive` copies with `CARGO_TARGET_DIR` = the lane's target; local update server `updserver.py` stands in for GitHub releases | the branch stays clean; the patch never enters it | `build-old.sh`, `build-new.sh` |
| Downgrades simulated by replacing the bundle at the same path (as a reinstall would) | 0.43's own "Install <tag>" is hardcoded to GitHub | — |
| Some dock shells were opened/written over the socket by me (`wsclient.py --script`, the same `shell_open`/`shell_write` the page sends), marked CRASH-PRE, SERVE-SHELL-1, ORPHAN-2 | no scripted clicks; deterministic setup for 7b/7d | `evidence/check-04-crash`, `check-07-downgrade-crash` |

## The relaunch trap

Both bundles lock **tauri 2.11.5**, plugin-process 2.3.1, plugin-updater 2.10.1.
- `tauri::process::restart` on macOS reads `Contents/Info.plist` `CFBundleExecutable`, then runs `Command::new(Contents/MacOS/<exe>).spawn()`. That is a direct spawn that inherits the environment, not LaunchServices. Confirmed empirically: each relaunched app had PPID 1 (its PGID was the old app's) and `HOME`/`TMUX_TMPDIR`/`TMPDIR`/proxies intact.
- The JS `relaunch()` path: `commands::restart` → `request_restart()` → `ExitRequested(RESTART_EXIT_CODE)` → `Exit`. The app's Exit handler runs (0.42.0: rings and cwds saved, dock shells killed; 0.43.x: `stop_owned`), then `restart`.
- Updater install (`updater.rs:1217`):
  1. Extract the tarball, top dir stripped, into a `$TMPDIR` tempdir (seen: `$R/tmp/tauri_updated_app*`).
  2. `rename` the running bundle into `tauri_current_app*` (deleted on drop).
  3. `rename` the new bundle into place.
  4. `touch` it.

  The running app's and serve's executables are deleted under them, which is harmless on macOS. The tarball keeps the sidecar's `rwxr-xr-x`.

## Checks

| # | Check | Expected | Observed | Evidence | Result |
|---|---|---|---|---|---|
| 1 | Merge + gates | green | 1 textual (CHANGELOG `### Added`) + 2 semantic conflicts (bare `Command::new` for serve in engine.rs; childenv-check vs `serve/tests.rs`). Gates on d4ce51b via `worktrees run --slot gate`: release build, bats 538/538, lint, core 879 / cli 129 / api 193 / app 35, tsc, cargo check, transport 204, 48 `*-check.mjs`, test-frontend 48, pnpm build, **serve_bin with tmux off PATH 11/11** — all green, 0 warnings | `sidecar-merge.md`, `gates-d4ce51b/summary.txt` | PASS |
| iso | Isolation | everything under `$R` | config `$R/Library/Application Support/<rehearse id>`; logs `$R/Library/Logs/…`; serve.log `$R/.cache/worktrees/serve.log`; inbox `$R/.cache/worktrees/inbox`; tmux `$R/tmux/tmux-501/default`; serve's env inherited (`ps eww`). Real serve.log unchanged (mtime 00:38, 0 "rehearse" lines), real app.log 0 "rehearse" lines, no rehearse dirs in real App Support/Logs, real tmux server has no demo/pre sessions, real registry untouched, `/Applications` and `~/.local/bin` untouched. **Only WebKit escaped**: it ignores `$HOME` (NSHomeDirectory) and created real `~/Library/Caches/<rehearse id>` and `~/Library/WebKit/<rehearse id>`; both removed at cleanup on (main)'s word | `evidence/01-real-baseline.txt`, `evidence/99-cleanup/real-final.txt` | PASS (WebKit note) |
| 5 | Update route | real updater, env kept | User: Settings → Version → Check for updates → "Update app → v0.43.0". Log: 14:36:56 `HEAD /releases/latest → 302 /tag/v0.43.0`; 14:37:01 `GET /latest.json`, `GET worktrees-server-0.43.0.app.tar.gz` (UA `tauri-plugin-updater/2.10.1`); old app 33481 exits 0; new app 55080 at 14:37:02.09 | `updates/server.log`, `evidence/07-update-watch.clean.txt` | PASS |
| 6.1 | Engine start | app.log names the sidecar, "no serve.json → starting one"; argv `--owner app --config-dir <rehearse>`, no `--allow-origin`; serve.json 0600 owner app; client over tauri://localhost; timing | all as expected. **app start 14:37:02.09 → serve 02.68 → serve.json 02.95 → page connected 02.99 (< 1 s)**; click → new window ≈ 6 s including the 11 MB download | `evidence/check-01-first-0.43.0/` | PASS |
| 6.2 | tmux | tmux server + stand-in pids unchanged, file keeps growing | tmux 39370 and stand-in 41422 (started 12:36) survived both updates, every quit, the crash and both downgrades; `standin.log` 9848 stamps 12:36:55 → 15:23:36, one 10 s gap at 14:28:34 while no rehearsal app ran (machine load), none at 14:37:01 or 15:07:52 | `evidence/99-cleanup/cleanup.txt` | PASS |
| 6.3 | Places via independent client | `list_workspace` = v0.42.0 `ls --json` | independent client (`wsclient.py`: handshake v1, token from the rehearsal serve.json, Origin `tauri://localhost`, serve's proof verified before sending): 3 places on both sides; the only differing shared field is `feat-alpha.declared` (`{last_opened_epoch}` vs `null`), which is the app path merging the declared store — v0.42.0's in-process `list_workspace` did the same (`lib.rs:383`); `ls --json` never does. Serve-only fields: `agent_sessions`, `profile_name`, `profile_stale`, `unborn` | `evidence/check-01-first-0.43.0/compare.txt` | PASS |
| 6.4 | Dock | cwd, `── restored ·` seam, REHEARSAL-1 in scrollback, history | on disk: shell-cwds `{feat-alpha: {"1": "/private/tmp"}}` unchanged across the update; the ring holds REHEARSAL-1/2, then the seam, and serve's new zsh starts in `/private/tmp`; per-tab history intact and appended after the update (REHEARSAL-3/4/5, CRASH-PRE). User: seam, REHEARSAL-1/2, `/tmp` prompt and ↑ order all correct. REHEARSAL-3 "missing" in batch 3 is explained by the ring bytes: in batch 2 only ↑/↓ recalls reached the shell, ending on a cleared line (no Enter, no `^C`); REHEARSAL-3 arrived at 14:58 as a bracketed paste. Nothing lost | `evidence/08-ring-at-quit.bin`, `check-01-*` | PASS |
| 6.5 | Host policy | host-policy.json seeded; "host-policy: copied …"; ui-state still has the keys | host-policy.json `{codex_permissions: ask, pi_project_trust: ask}`; serve.log "host-policy: copied codex_permissions from ui-state.json (once; the host owns it from now on)" ×2; user: Settings shows Ask / Let pi ask. **ui-state.json no longer has either key** — 0.43's `loadSettings` deletes them (deliberate) and the whole-blob save writes that out | `evidence/check-01-first-0.43.0/ui-state.diff.txt` | FAIL → D6 |
| 6.6 | Settings, window, What's new | only expected keys change; What's new 0.43.0 | ui-state diff: `last_seen_version 0.42.0 → 0.43.0` (What's new had a section and was dismissed), `update_dismissed "" → "v0.43.0"`, `usage_kimi` added; the two host-policy keys removed (6.5). Window restored at its saved frame. Version 0.43.0 (user). 0.43.1: What's new named 0.43.1 (user) | same | PASS (except 6.5) |
| 6.7 | Logs | app.log shim-only; serve.log at `$R/.cache/worktrees/serve.log`; `log_info`/`log_tail`; Settings → Logs | `Logs/<rehearse id>/app.log` holds only `engine:` and `window restore` lines; the `startup v0.43.x` line goes to `Logs/net.casadelvalle.worktrees/app.log` (the default id — known, AGENTS); `log_info` → `{dir: $R/.cache/worktrees, file: …/serve.log}`; user: Settings → Logs shows `~/.cache/worktrees/worktrees/rehearse-server-edition-update/home/.cache/worktrees/serve.log` | `…/log_info.json` | PASS |
| 6.8 | Mixed versions | v0.42.0 CLI/MCP ↔ serve, both directions; v0.42.0 `worktrees mcp` alongside | v0.42.0 `worktrees mcp --mutations` (serverInfo 0.42.0, 16 tools) beside serve: `set_note`, `set_lifecycle saved`, `report` (from feat-alpha) all ok; v0.42.0 `new feat-gamma` → serve's `list_workspace` shows all three. Reverse: serve `set_note`, `set_lifecycle`, `new_place feat-delta` → v0.42.0 `ls` lists feat-delta (live), `status` reads `lifecycle_effective: saved` and `note: note-by-serve-0.43.0` | `evidence/check-02-mixed/` | PASS |
| 6.9 | Inbox | v0.42.0 `worktrees show` opens in the new app | 14:41:47 `show docs/notes.md` → serve 14:41:48 "inbox: open"; user: shown in the dock's Files tab AND the viewer opened with "# Notes / line 1" | `evidence/check-03-inbox/` | PASS |
| 7a | Clean quit | stopping → stopped; serve.json removed; ring flushed; tmux + stand-in alive | user ⌘Q (no dialog): serve.log 14:59:43 "stopping" → 14:59:44 "stopped"; app.log "serve pid=55127 exited (exit status: 0)", "quitting; serve pid=55127 stopped"; serve.json gone at 14:59:44; ring kept (last dirty tick 14:59:23 held REHEARSAL-4; nothing dirty at exit); dock zsh gone; tmux + stand-in alive. Same again for 0.43.1 at 15:08:48 | `evidence/08-quit-watch.txt` | PASS |
| 7b | Crash | `kill -9` app: serve + dock shell survive; relaunch adopts; the next quit stops it | `kill -9` 25335 at 15:02:22 → serve 25439 PPID 1, dock zsh 27148 alive, serve.json kept → relaunch 28298: "observed serve(pid 25439 …): proven v0.43.0" → "adopting … this app stops it on quit"; user: the same terminal, no extra seam (the shell was re-attached, not respawned). The next exit (the 0.43.1 update) stopped the adopted serve (7c) | `evidence/check-04-crash/crash.txt` | PASS |
| 7c | Later update 0.43.0 → 0.43.1 | Exit hook or Replace? no SIGKILL; rings kept | 15:07:52 updater; **the Exit hook stopped the ADOPTED serve**: "engine: quitting; serve pid=25439 stopped", serve "stopping/stopped". New app 73672: "observed no serve.json → starting one" — **not** Replace. No SIGKILL or "replacing" in any log. Ring kept, with exactly 3 seams (update to 0.43.0, relaunch after ⌘Q, update to 0.43.1) and none for the crash; Full access carried over (serve 73753 boots `codex_permissions=full`) | `evidence/11-update-0.43.1-watch.txt`, `check-05-update-0.43.1/` | PASS |
| 7d-clean | Downgrade after a clean quit | 0.42.0 doesn't see a 0.43 host-policy change | Codex set to Full access in 0.43.0 (host-policy.json only); after clean quit, v0.42.0 put at the same path. **User: Codex = Auto-review** — not Full access (expected), but also not the user's original Ask: ui-state had no key, so 0.42.0 took its defaults and wrote them back (`auto-review` / `never`). Its dock replayed the 0.43.1-written ring (REHEARSAL-5 + seam, `/tmp`), DOWNGRADE-1 ran; no What's new | `evidence/check-06-downgrade-clean/` | PASS for the check, exposes D6 |
| 7d-crash | Downgrade after a crash | record two engines | 0.43.1 serve 2217 with a dock zsh (3081, `/private/var/tmp`); `kill -9` app; v0.42.0 put at the same path and launched (5242). **Two live shells for one tab**: v0.42.0 replayed serve's ring (SERVE-SHELL-1 + seam 15:15:14) and spawned its OWN zsh 40502 in the cwd serve saved (user: `pwd` = `/private/var/tmp`), while serve's 3081 kept running, invisible to it. **Doubled polls**: both run the 3 s host poll and status checks. **Inbox**: 6 `worktrees show` → serve took 4 (lost, no client), v0.42.0 took 2. **Who writes shell-cwds.json and the ring**: both, same key — after v0.42.0's clean quit saved DOWNGRADE-2, the orphan's next 15 s tick (15:22:46) rewrote `scrollback.bin` (DOWNGRADE-2 gone from disk) and `shell-cwds.json` (`/private/etc`). serve stopped clean on SIGTERM | `evidence/13-two-engines-watch.txt`, `check-07-downgrade-crash/` | recorded → D2, D7 |
| 8 | Linux AppImage mount | — | answered from sources (§8) | — | NOT COVERED (no Docker test: a Linux AppImage build is not cheap) |

## 8. Linux AppImage: does the mount go away under an app-owned serve?

**No, not while serve runs. That is by inheritance, not by design.**
- **AppImage type2-runtime `runtime.c`.** `pipe(keepalive_pipe)` is created without `O_CLOEXEC`. The forked FUSE daemon keeps the WRITE end and writes until `EPIPE`, then calls `kill(fuse_pid, SIGTERM)` and unmounts. The parent keeps the READ end and `execv`s AppRun. So every process that inherits that fd keeps the mount alive.
- **AppImageKit `AppRun.c`** (tauri's `apprun-old`). It does `chdir(usr_in_appdir)`, puts the AppDir first on `PATH`/`LD_LIBRARY_PATH`, then `execvp`. It closes no descriptors.
- **The app → serve.** The app spawns serve through `std::process::Command`, which never closes inherited fds, so serve holds the read end.
- **tmux.** `server.c`/`proc.c` call no `closefrom`; only `client_exec` does. A tmux server started by the app or serve also holds the fd, so the mount lives as long as tmux does. v0.42.0 behaved the same: its app started tmux.
- **Build tooling.** tauri-bundler (linuxdeploy @ 07333c6 + linuxdeploy-plugin-appimage continuous) sets `APPIMAGE_EXTRACT_AND_RUN=1` only to run linuxdeploy at BUILD time. The shipped AppImage uses the FUSE path.
- **Hazard: extract-and-run mode.** It is opt-in (`APPIMAGE_EXTRACT_AND_RUN` or `--appimage-extract-and-run`), never automatic. The runtime forks the app, `waitpid`s it, then runs `rm_recursive(prefix)` unless `NO_CLEANUP` is set. That deletes the extracted tree under a serve that outlived the app: its exe is unlinked and `WORKTREES_VIEWER_JS` is gone.
- **`tauri-utils` 2.9.3 `resource_dir` on Linux.** `<exe>/../lib/<productName>` if it exists, else `$APPDIR/usr/lib/<name>`, else `/usr/lib/<name>`. This matches `sidecar.sh`.

## Defects (owner, severity)

| # | Severity | Owner | Defect | Evidence |
|---|---|---|---|---|
| D1 | **medium** | **#513** (× #516) | Under serve, `profile::worktrees_bin()` returns `current_exe()` because the sidecar is named `worktrees`, and that is the **app-private** CLI. So `mcpsetup::install` would run `claude mcp add -s user worktrees -- "<bundle>/Contents/MacOS/worktrees" mcp --mutations` into the user's GLOBAL Claude config; the same goes for `codexmcp`, kimi's `mcp.json` and `guidance::guard_bin`. `mcp_status` reports the sidecar and can never say `cli-missing`. This contradicts sidecar.sh ("nothing outside the bundle points at this copy") and the CHANGELOG ("keep working whether the app is installed or not"). On an AppImage the path is a per-launch `/tmp/.mount_*`. v0.42.0 reported `~/.local/bin/worktrees` | serve.log on the real update path, 20:37:02Z: `mcp_status … worktrees=…/Worktrees Server.app/Contents/MacOS/worktrees` (v0.42.0 app.log: `worktrees=~/.local/bin/worktrees`); `profile.rs:1208`, `mcpsetup.rs:398-410` |
| D6 | **medium** (security-relevant on rollback) | **#508** (c25cc2a) | 0.43's `loadSettings` deletes `codex_permissions`/`pi_project_trust` and the whole-blob save drops them from ui-state.json. v0.42.0 reads only ui-state.json, so **any downgrade silently resets Codex permissions to Auto-review — wider than the user's Ask** — and pi trust to Skip, and writes those defaults back. Keeping the client copy (unused by 0.43 under host policy) would preserve the choice on rollback. Measured: Ask (0.42.0) → host copied → keys deleted → Full access (0.43, host only) → downgrade shows Auto-review | `check-01-first-0.43.0/ui-state.diff.txt`, `check-06-downgrade-clean/ui-state.txt`; settings.ts:796-797 |
| D7 | **medium** | **#524** (+ release notes) | After a crash an app-owned serve lives on with no client, indefinitely. If the user then downgrades (or runs any in-process-engine app), **two engines own one config dir**: two live shells for one dock tab, and the orphan's 15 s tick **overwrites the dock ring and shell-cwds.json** the old app just saved (DOWNGRADE-2 lost from disk). Nothing tells the user an engine is still running. Options: an orphan serve with no clients for N minutes stops itself; release notes say "quit Worktrees before downgrading" | `evidence/13-two-engines-watch.txt` 15:22:24 → 15:22:46; `check-07-downgrade-crash/ring-after-orphan-tick.bin` |
| D2 | low | **#516** | serve drains `~/.cache/worktrees/inbox` every 3 s with ZERO clients (`host.rs:208`). After an app crash, `worktrees show` is consumed in < 1 s and emitted to nobody, so it is lost. v0.42.0 with the app down left a request 30 s for the next launch. With two engines it's a race: 4 of 6 went to the orphan | `check-07-downgrade-crash/inbox.txt`, `preflight-downgrade/inbox.txt` |
| D3 | low | **#524** | Fresh install only: the shim saves window state before the config dir exists ("window state: could not save: No such file or directory"); the dir is now created by serve. Not on the update path | `evidence/preflight/check-1/` |
| D4 | docs | **#513** | CHANGELOG item and `sidecar.sh` header say the app "will start" serve; on the stack it does | `sidecar-merge.md` |
| D5 | note (retarget) | **#513** | On retarget, `engine.rs` must spawn serve through `childenv::command` (resolved in this merge). serve then runs with `APPDIR` scrubbed, so its own childenv is a no-op; that is correct only because its env and cwd are already clean | `sidecar-merge.md` |
| N1 | note | release notes/docs | WebKit's storage ignores `$HOME`; any HOME-isolated run (sandbox.sh too) writes real `~/Library/{Caches,WebKit}/<identifier>` | iso row |
| N2 | note | (pre-existing, both versions) | The app's `new_place` with no provider defaults to the claude harness and ignores `ai_cmd = none` (v0.42.0 `lib.rs:1491`, 0.43 `ops.rs:587`). Hit here by my own client passing `null`; the stray claude was killed and wrote only under `$R` | `check-02-mixed/feat-delta-*.txt` |
| N3 | note | (pre-existing) | `claude_status` curl runs twice on every "window visible" in both versions | serve.log, app.log |

## Process inventory at the end

serve 2217 was stopped with SIGTERM ("stopping/stopped"). The private tmux server and the stand-in were killed through `tmux -S $R/tmux/tmux-501/default kill-server`. The update server and all watchers are stopped. `pgrep` finds nothing under `$R/Apps`, `home-pre` or the lane path: see `evidence/99-cleanup/cleanup.txt`. The two real `~/Library` `.rehearse` dirs were removed after a path check. `$R`, `home-pre/`, the `bundles/` (0.42.0, 0.43.0, 0.43.1) and all evidence are kept.

---

# Sidecar merge — preview of retargeting #513 onto feat/server-edition

Rehearsal lane `rehearse/server-edition-update`, 2026-10-21.

- Base: `feat/engine-auto-reconnect` @ 2b3c359 (stack #508 → #516 → #515 → #524 → #525)
- Merged: `origin/feat/sidecar-bundle` @ 6e75157 (#513)
- Merge base: eb0f72e ("Merge origin/main (#520 …) into the worktrees-api extraction")
- Merge commit: b161a63 (local, signed, never pushed)
- Then: d4ce51b "chore: 0.43.0 — REHEARSAL ONLY" (Cargo.toml, install.sh
  SCRIPT_VERSION, Cargo.lock members, `## [0.43.0]` cut from `[Unreleased]`)

## Files touched by both sides

`AGENTS.md`, `CHANGELOG.md`, `crates/worktrees-api/src/hostenv.rs`,
`crates/worktrees-api/src/term.rs`, `crates/worktrees-core/src/tmux.rs`.
Only `CHANGELOG.md` conflicted textually; the other four auto-merged into
disjoint hunks (hostenv: the stack added `APP_IDENT`/`default_config_dir`,
#513 routed `fixup_gui_path`'s login shell through childenv; tmux.rs: the
stack's exact-target `tune_session` + its test, #513's `tmux_cmd()`).

## Conflicts and resolutions

| # | Where | Kind | What | Resolution |
|---|---|---|---|---|
| 1 | `CHANGELOG.md` `[Unreleased]` → `### Added` | textual | Both sides opened `### Added` and added items at its top (stack: reconnect, serve, dev:serve; #513: CLI sidecar, Linux app). | Kept both item lists, stack first, under ONE `### Added`. Checked: one `### Changed`, one `### Added`, one `### Fixed` between `[Unreleased]` and `[0.42.0]` (AGENTS.md's two-headers trap). |
| 2 | `app/src-tauri/src/engine.rs:633` (`Engine::start`) | semantic — caught by #513's `childenv-check.mjs` | The shim spawned serve with a bare `Command::new(&bin)`. #513's rule: every spawn goes through `worktrees_core::childenv`. On an AppImage, serve would have inherited the AppDir's `LD_LIBRARY_PATH`, `PATH`, etc. and a cwd inside the mount. | `worktrees_core::childenv::command(&bin)` (the app crate already depends on core); `std::process::Command` dropped from the module's `use` and imported by `mod tests` itself (its one `Command::new("sleep")`). **Consequence worth knowing:** serve then starts with `APPDIR` removed, so serve's own `childenv::appdir()` is `None` and it does no scrubbing of its own — correct only because the env it inherits is already scrubbed and its cwd is `$HOME`. Its EXECUTABLE still lives in the mount (see report §8). |
| 3 | `app/scripts/childenv-check.mjs` vs `crates/worktrees-cli/src/serve/tests.rs:748` | semantic — check vs layout | #516 keeps serve's tests in their own file (`#[cfg(test)] mod tests;` in `serve/mod.rs`). The check only recognises an inline `#[cfg(test)] mod x {`, so it read all of `tests.rs` as product code and failed on `Command::new("git")` in `scratch_repo`. | The check skips a `tests.rs` whose sibling `mod.rs` declares `#[cfg(test)]\nmod tests;`. (Alternative: route the test's git through childenv — rejected, it would make #516's test code depend on #513.) |

After 2 and 3: `childenv-check.mjs` "101 files, 2 pty builders checked — ok";
`engine-route-check`, `api-boundary-check`, `hostpolicy-check`,
`serve-dispatch-check` all good.

## Wording that goes stale on retarget (not changed here)

- `CHANGELOG.md` (#513's item): "used only by the app (which will start
  `worktrees serve` with it)" — on the stack it already does.
- `app/scripts/sidecar.sh` header: "for the app's use only (it will start
  `worktrees serve`; …)" — same.

## Gates

See `gates-d4ce51b/summary.txt` and the report.
