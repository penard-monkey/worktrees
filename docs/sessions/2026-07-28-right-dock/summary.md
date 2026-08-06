---
title: "Session: right dock — file browser, editable viewer, embedded terminals"
---

# Session: right dock — file browser, editable viewer, embedded terminals

- **Dates:** 2026-07-27 → 2026-07-28
- **Worktree:** `.worktrees/ui-changes` (branch `ui-next`)
- **PR:** [#56](https://github.com/penard-monkey/worktrees/pull/56) (squash-merged → `eb6e54e`)
- **Release:** [v0.4.0](https://github.com/penard-monkey/worktrees/releases/tag/v0.4.0)
- **Planning files:** `planning.tar.gz` (this dir)
- **Pivotal screenshots:** `dock-files.png`, `multi-term.png` (this dir); full set in `~/.cache/worktrees/worktrees/ui-changes/`

## What shipped

A collapsible, resizable **right dock** in the Tauri app (⌘J, or the ▧ button in a place's toolbar), plus a drop to a **single-pane** app tmux session.

### Files tab
- Lazy worktree tree, `.gitignore`-filtered via `git check-ignore --stdin -z`.
- Editable viewer: plain mono `<textarea>` (no editor lib — project ethos), ⌘S / Save, dirty dot. Binary + oversized files stay read-only; "Editor" opens the external editor.
- Backend (`app/src-tauri/src/lib.rs`): `list_dir`, `read_file` (bounded via `File::open` + `take(cap+1)`, returns mtime), `write_file` (mtime compare-and-swap + atomic temp+rename preserving mode). All path-guarded to registered project roots via `guard_under_projects` (canonicalize + component-wise `starts_with`).

### Terminal tab
- One or more embedded shells: ＋ / ⌘⇧T to add, per-tab ✕, close-all → empty state. Only the active shell is mounted (tmux keeps the rest warm).
- Each shell = its own **sidecar tmux session** `<canonical>~term` / `~term~N` (`open_shell_session`); tabs restore from live tmux (`list_shell_sessions`); `close_shell_session` kills one.
- `crates/worktrees-core/src/tmux.rs`: `SHELL_SIDECAR_MARKER` (`~term`), `shell_sidecar_name/prefix/index`, `is_shell_sidecar`, `kill_shell_sidecars`, `session_names`. `session_in` skips sidecars so a bare shell can't be adopted as the AI session.

### Single-pane app session
- `crates/worktrees-core/src/ops.rs`: `launch()` gained `spare_shell: bool`; `cmd_open` gained `--no-spare`. App open paths pass single-pane (Claude full width); CLI `new` still splits its deps-install pane.

### Frontend
- `app/src/App.tsx`: module-scope `TreeNode`/`FileTree`/`FileViewer`/`DockTerminal`/`TerminalTabs`; dock is a 4th grid column gated on `dockShown`; ⌘J toggle + ⌘⇧T; keyed off `repo|slug`.
- `app/src/settings.ts`: `dock_open`/`dock_width`/`dock_tab` + `clampDock` + `--dock-w`.
- `app/src/App.css`: `.dock*`, `.filetree/.tree-*`, `.viewer*`, `.termtab*`.
- `app/src/mock/install.ts`: virtual FS + sidecar tracking + mtime CAS so the dock drives headlessly (Playwright).

## Decisions (with why)

- **Embedded terminal = sidecar tmux session, NOT grouped.** Simpler, matches "a separate terminal" mental model, honors "terminals ATTACH to tmux, never own shells." Grouped sessions would duplicate panes in `list-panes -a` and pollute derived state.
- **Editable viewer via `<textarea>`, not CodeMirror.** Project rule: no UI libraries. Existing "Open in editor" remains the escape hatch. Syntax highlighting deferred.
- **Single-pane scoped to the app, not the CLI.** `new.bats`/`real-tmux.bats` assert `new` makes 2 panes; gating on an explicit `--no-spare` flag (not on `install_cmd` being empty) keeps the CLI + bats untouched.
- **Version bump 0.3.2 → 0.4.0 (minor).** New feature per SemVer.

## Dead ends / gotchas (highest value)

- **Sidecar marker `-term` was a BLOCKER collision (found by fable review).** Slugs are slugified git refs and `-` is legal, so a place on branch `long-term` gets session `<prefix>-long-term` — byte-identical to the sidecar of a place named `long`. `kill_shell_sidecars`/`list_shell_sessions` would then kill/list another place's **live Claude session**. Fix: marker `~term` — git ref names forbid `~` (tmux only forbids `.`/`:`), so no real place session can contain it. Collision-proof by construction; unit test pins it. **Lesson: never derive an internal namespace marker from a character that can appear in user-controlled slugs.**
- **Sidecars must be named backend-side from repo+slug, not the frontend's `tmux_session.name`.** That name can be an *adopted foreign* session; naming sidecars off it leaked orphaned sessions and churned tab identity when the canonical session came up. The webview also shouldn't be able to name/kill arbitrary sessions. Fixed: `open/list/close_shell_session` take `repo`+`slug`, derive canonical name + cwd.
- **Teardown must be gated on op success.** Original app-layer `kill_shell_sidecars` ran even on a *refused* `rm` (dirty worktree), destroying the user's shells. Moved teardown into core `cmd_close`/`cmd_rm` on the kill/success path — also fixes the CLI-side leak (CLI never swept sidecars before).
- **`read_file` loaded the whole file before capping** — clicking a multi-GB artifact allocated it entirely. `File::open` + `take(cap+1)`.
- **Blind last-writer-wins save** in an app whose premise is Claude editing the same tree. Added mtime compare-and-swap (`write_file` refuses if disk changed) + atomic write + viewer conflict/Reload + auto-reload-when-clean on `places:changed`.
- **`open_shell_session` exists-then-create race** fires constantly in dev (React StrictMode double-mounts effects). On `new_session` failure, re-check `session_exists` and treat as success.
- **`⌘⇧T` handler** ran before the `switchOpen` palette guard (added a tab behind the ⌘K scrim) and intercepted `Ctrl+Shift+T` inside the terminal. Require `⌘` (not ctrl); gate on palette-open.
- **`_tmp/` iCloud symlink is unreadable by this process** (macOS TCC) — couldn't read the reference image; designed from the user's description. Not a code issue, but blocks image review from that path.
- **Merge with main was non-trivial:** main shipped v0.3.2 (quick switcher) + per-project settings (#58) while this branched. `close_one`/`close_place` were refactored on main (name-bound consent); re-applied the sidecar sweep onto main's structure. CHANGELOG needed hand-untangling (quick switcher → released 0.3.2; single-pane Changed → 0.4.0).

## Verification

- Gates (post-merge, at 0.4.0): `cargo build --release -p worktrees-cli`; `cargo check -p app`; `tsc --noEmit`; **137** core tests (incl. collision-proof `shell_sidecar_*` tests); `make lint`; **237** bats (incl. 2 new `--no-spare` tests).
- PR #56 CI: 9/9 green (app/rust/test/install/lint × macOS + Ubuntu).
- Playwright vs `dev:mock`: file open → edit → dirty → Save → clean (mtime CAS round-trip); multi-terminal add(＋/⌘⇧T)/switch/close/empty-state; `~term` marker confirmed in the live banner.
- Release: v0.4.0 workflow green — 4 CLI targets + 2 signed app bundles + `latest.json` + `checksums.txt` published (not draft).
- Console `dimensions` error under headless xterm is a pre-existing quirk (fires for the main terminal too), not a regression.

## Follow-ups

- Syntax highlighting in the viewer (deferred — would need CodeMirror, conflicts with the no-UI-libs rule; revisit if editing becomes a real workflow).
- External-kill edge: if a sidecar is killed outside the app (bare `tmux kill-session`) while its tab persists, clicking the tab recreates it. Benign (scratch shell), not handled.
- `close_one`'s "nothing to close" path sweeps sidecars, but a place whose AI session is down while sidecars linger and is never closed keeps them until `rm`. Acceptable.
