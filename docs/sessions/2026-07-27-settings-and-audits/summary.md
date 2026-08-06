---
title: "Session: settings evaluation, nav audits, ⌘K, v0.3.0 + v0.3.1"
---

# Session: settings evaluation, nav audits, ⌘K, v0.3.0 + v0.3.1

- **Date:** 2026-07-27
- **Worktree:** `.worktrees/everything-settings`
- **Branches:** `fix/menu-defects`, `feat/settings-tier-a`, `fix/silent-failures`,
  `feat/behavior-settings`, `feat/auto-fetch`, `feat/remove-branch-diag-ai`,
  `fix/busy-dot-claude-state`, `release/v0.3.1`, `feat/quick-switch` (all merged + deleted)
- **PRs:** #41, #42, #43, #45, #46, #47, #49, #50, #51, #52
- **Releases:** `v0.3.0`, `v0.3.1` (both built + signed; latest.json published)
- **Planning files:** `planning.tar.gz` (this dir) — task_plan / findings / progress
- **Screenshots:** `busy-dot-states.png`, `quick-switch-ranked.png` (this dir);
  full set in `~/.cache/worktrees/worktrees/everything-settings/` + `.../quick-switch/`

## What shipped

### Settings pane (Tier A + Tier B from the evaluation)
- **Keyboard:** ⌘, opens Settings; ⌘1 Home, ⌘2/3/4 lens jump (reveal-not-toggle
  via `selectLens`); ⌘E open-in-editor; read-only Shortcuts cheatsheet
  (`SettingsSheet.tsx`). ⌘K quick switcher (see below).
- **Commands section:** `ai_auto_resume` toggle (bidirectional ctx override
  "Open fresh"/"Open with resume"); external terminal command (`{session}`
  token → "Open in terminal app" ctx item); read-only AI-config display +
  reveal-config (phase 1). `App.tsx` `open_terminal`, `open_editor` (both `sh -c`).
- **Git section:** background auto-fetch (Off/5/15/60 min; `lib.rs`
  `claude_activity`-adjacent watcher, `fetch_origin_root`, hardened:
  `GIT_TERMINAL_PROMPT=0` + ssh BatchMode + 60s deadline) + "Fetch origin" verb.
- **Startup:** `restore_last` (selection-only). **Version:** `update_auto_check`.
  **Logs:** offline "Copy diagnostics". **Data:** reveal settings file + two-click reset.
- **Cut:** dead `window_w/h` inputs; dead `up_cmd` schema.
- **Removed defect:** topbar ⋯ arm leak; remove-project no-confirm (both surfaces);
  copyText silent failure; remove-label inconsistency.

### Silent-failure batch (core error plumbing, `crates/worktrees-core`)
- tmux `new_session` → `Result<pid, String>`; `launch()` returns rc + `ui.error`
  (was silent "Session ready" on failure). `ops.rs`, `tmux.rs`, `lib.rs`.
- Adopted (foreign-named) sessions read UP via prefetched `tmux::PaneList` (one
  `list-panes -a`/snapshot); main excludes `.worktrees/` panes. `project.rs`.
- `new_place` returns final slug in `CmdResult` (`Project::resolve_new_slug`);
  frontend stops guessing. `git_status_captured` surfaces git's real stderr.

### Busy dot rewrite (`fix/busy-dot-claude-state`, #49)
- Was tmux `#{session_activity}` = client attach/keypress → decayed while Claude
  worked. Now reads `~/.claude/sessions/<pid>.json` probes: status busy → green
  blink, waiting → amber static, idle/shell → none. Keyed by `cwd == place.path`.
  `lib.rs` `claude_activity()` + `pid_alive()` (libc); `--warn` token per theme.

### ⌘K quick switcher (`feat/quick-switch`, #52)
- Module-scope `QuickSwitch` palette; fuzzy subsequence rank over
  slug+branch+project+note (slug-biased, recency tiebreak, empty→recent);
  Arrow/Home/End/Enter/Esc; busy/waiting dots; chord-guard while open; works
  rail-only; reuses `enterPlace`. Frontend-only. `App.tsx`, `App.css`.

## Decisions (with why)
- **Opus agents implement, this loop reviews before merge** — user directive to
  save Fable tokens; the independent-implementer/reviewer split caught a real
  blocker (see below). See memory `opus-implements-fable-reviews`.
- **remove-deletes-branch = per-action armed pair, NOT a setting** — force stays
  false so `git branch -d` only deletes merged; safe by construction, no hidden state.
- **auto-fetch default Off** — first background network actor; opt-in only.
- **busy-dot liveness = PID-alive, NOT updatedAt age** — probe file is rewritten
  on status *transitions*, so `updatedAt` is minutes-stale while genuinely busy
  (empirically: a live busy session's file was 84s old). Age-window would drop
  long-running sessions.
- **ai-command phase 1 only** — editable field needs a comment-preserving config
  writer in core (`cfg_set`) that doesn't exist; deferred.
- **global-summon-hotkey deferred to v0.3.2** — bundles with ⌘K switcher.
- **Dropped as already-upstream:** reshow-whats-new (#40), system-theme-pair (#38).

## Dead ends / gotchas (highest value for future sessions)
- **PR3 main false-adoption blocker** (caught in review, not by gates): the
  session-adoption fallback matched `path.starts_with("{wt}/")`; worktrees nest
  under `main_root/.worktrees/`, so the MAIN place adopted any worktree's session
  whenever main was down + any worktree up (the common state). Fix: `session_in`
  gained `exclude_under`; main passes `wt_root`. Gates were green THROUGHOUT — a
  bats/tsc-green diff can still be semantically wrong; the human-review gate earned
  its place here.
- **Upstream drift mid-stream, repeatedly:** origin/main moved under us (#38/#40
  before we started; #44 docs between PRs). Always re-fetch + re-locate by grep,
  never trust audit line numbers across PRs (App.tsx shifted ~90+ lines).
- **`session_activity` was the wrong tmux field entirely** — it tracks client
  attach/keypress, not pane output. Verified empirically (busy sessions 434-477s
  stale while spinning). The right signal was Claude's own probe files; other
  tools (craftzdog, pbauermeister) read the same. Lesson: verify the signal
  empirically before building on it.
- **`updatedAt` freshness trap** (busy dot): the naive "fresh within 60s" guard
  the design started with would have re-introduced the exact dropout being fixed.
  Empirical sampling on live sessions caught it → PID-alive instead.
- **macOS has no `timeout`** — use direct runs or `gtimeout`; a wrapped
  `claude agents --json` silently didn't run.
- **Release branch divergence:** a leftover remote `settings-next-2` caused a
  non-fast-forward on the v0.3.1 push; cut from a clean `release/v0.3.1` instead
  and deleted the stale remote. Don't reuse-and-reset a branch that has a remote.
- **Model switched Fable→Opus mid-session** (Fable usage limit) during the
  busy-dot investigation; the workflow's final verify agent died on the limit and
  Opus finished the synthesis inline. Long autonomous streams can outrun a tier's quota.

## Verification
- Every PR: full gate suite — `cargo build --release -p worktrees-cli` (first),
  `make test` (bats 134), `make lint`, `cargo test -p worktrees-core` (9), app
  `tsc --noEmit` + `cargo check -p app`. All green at each merge.
- Two audits ran live against the mock harness via Playwright (ctx-menu audit +
  each UI PR); busy-dot and ⌘K validated live with screenshots (this dir).
- Signal claims (session_activity staleness, probe-file semantics, `claude agents
  --json`) verified empirically on live sessions, not from docs.
- v0.3.0 + v0.3.1 release workflows succeeded; all 10 assets each verified
  (CLI ×4, signed app ×2 + .sig, latest.json, checksums).

## Follow-ups (swept to ROADMAP.md)
See `ROADMAP.md` → "Nav / settings backlog (from 2026-07-27 audits)". Highlights:
2 confirmed bugs still open (note-focus-terminal-steal, github-url percent-encode),
~9 ux-gaps, origin/HEAD base detection, mock fault-injection, ~26 polish items,
global-summon-hotkey (v0.3.2), ai-command phase 2. Full detail lives in the
archived `planning.tar.gz` (findings.md).
