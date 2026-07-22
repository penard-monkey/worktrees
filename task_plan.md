# Task Plan — Evaluate forking amux / emdash / coder / mux vs. our `worktrees`

## Goal
Decide whether to fork one of four upstream tools (mixpeek/amux, generalaction/emdash,
coder/coder, coder/mux) and re-shape it around THIS project's principle —
**"a worktree is a PLACE, a branch is work that flows through it"** — while keeping
whatever those tools do well. Or conclude: don't fork; cherry-pick ideas into our
own tiny bash CLI. Deliverable = a reasoned recommendation, not code.

## Our principles (the yardstick) — see findings.md for detail
1. Worktree = PLACE: persistent dir + tmux session + AI CLI history + node_modules.
2. Branch = flows THROUGH the place; `switch` never tears down the expensive state.
3. One tmux session per place: pane 0 = your AI CLI, pane 1 = deps install + shell.
4. Zero-config, works in ANY git repo, no daemon/server, bash-3.2 + macOS + Linux.
5. Thin, transparent, single ~680-line bash file. DWIM commands. Loud guards.
6. AI-CLI-first (claude/codex/opencode), local-only.

## Fork-fitness criteria (how each tool is judged)
- Language & size (maintainable by a solo/small project? ours = ~680 lines bash)
- Architecture: thin stateless CLI vs. daemon/server/Electron/cloud-platform
- Native git-worktree use vs. containers/VMs/cloud sandboxes
- Core abstraction: does it model workspace as PLACE, or as ephemeral task/session/env?
- Session model: tmux / web / Electron / SSH / container
- AI-CLI integration model
- License (fork-friendly?), maturity/activity
- "Does too much": what we'd have to rip out
- "Worth keeping": ideas worth stealing regardless of fork decision
- Alignment with the PLACE concept (1–5)

## Phases
- [x] Phase 1 — Grok our project's principles (read bin/worktrees + README) — COMPLETE
- [x] Phase 2 — Research each of the 4 tools — COMPLETE (2 workflow rounds; round 1 lost
      amux/emdash/coder to transient rate-limits, round 2 recovered them)
- [x] Phase 3 — Adversarially verify key claims per tool — COMPLETE (all 4, high confidence)
- [x] Phase 4 — Synthesize comparison matrix + fork recommendation — COMPLETE
- [x] Phase 5 — Write recommendation to findings.md, present to user — COMPLETE

## Decisions
- **DON'T FORK any of the four.** None clears all fork gates (worktree-native + thin scope
  + friendly license + net-positive economics). All center on an EPHEMERAL unit
  (session/task/provisioned-env), not our durable PLACE. Our ~680-line bash tool already
  owns the concept.
- Best-if-forced base = emdash (worktree-core + Apache-2.0), still only 2/5 — declined.
- Path forward: keep ours, cherry-pick 7 bash-able ideas (see findings.md). Copy ideas, not code.

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
| Workflow round-1 rate-limit (amux/emdash/coder) | 1 | Re-ran as round-2 workflow; all 4 verified |
| args.mux didn't inject into round-2 synthesis | 1 | Synthesized final 4-way comparison by hand from verified data |

---

# Part 2 — Build a Tauri UI app on top of the CLI engine

## Goal (revised direction)
Not a fork, not just cherry-picks. Build David's OWN place-manager UI app ("my flavor of
amux/emdash") where **the `worktrees` CLI is the engine** and the UI adds what the terminal
can't: **resurfacing** dormant places + an explicit **lifecycle** + resume-on-reopen +
**per-place infra spin-up** (port slots + docker stack), modeled on the CDV ancestor script.

## Key decisions
- **Stack = Tauri** (Rust core + web frontend). Chosen over Electron (bloat), local-web
  (needs daemon+browser, only for remote), TUI (not graphical enough for the vision).
- **CLI stays the engine.** Extend `bin/worktrees` with `--json` + `up`/`down`/`provision`/
  lifecycle subcommands. UI shells out; never re-implements git/tmux/docker.
- **Two state kinds:** derived (branch/dirty/ports/tmux/stack) computed live; declared
  (lifecycle label/pin/notes) in a plain JSON per repo. NO database.
- **Terminals ATTACH to tmux** (never own PTYs) — survives UI crash, still `tmux attach`-able.
- **Bring back CDV stack-mode** as a first-class, config-driven infra convention.

## Lifecycle states (declared, reconciled against live state)
Active · Idle · Closed · Saved/Pinned · Archived · Abandoned (see findings.md for the matrix).

## Phases
- [x] Phase 6 — Design workflow (4 facets design→critique + synthesis) — COMPLETE (all
      feasibility HIGH, all 5 principles upheld). Written to DESIGN.md.
- [x] Phase 7 — DESIGN.md written (spec + P0–P4 plan + first PR) — COMPLETE, pending David's go
- [x] Phase 8 — **P0**: `worktrees ls --json` (schema v1) CLI-only + `test/json.bats` — DONE +
      MERGED (PR #1, squash → main 8ef1854). CI green (lint, test macos+ubuntu, install×2).
      Fixed a bash-3.2-vs-5.x backslash-escaping portability bug caught by ubuntu CI (sed for
      the backslash step). Full suite 117/117 under BOTH bash 3.2 and 5.3.
- [x] Phase 9 — P1: Tauri app + embedded tmux terminal spike — DONE + MERGED (PR #2, squash →
      main c624ab8). Interactively VERIFIED by David (screenshot: live embedded tmux, reopen
      reattached, spike-demo shown "active"). The load-bearing risk is retired.
- [x] Phase 10 — P2: declared store + lifecycle — DONE (PR #3, branch feat/lifecycle; pending
      CI + interactive verification). Rust owns the declared store (read+write) + computes
      `lifecycle_effective`; CLI stays stateless/derived. Reconciliation: archived/abandoned
      sticky → saved → tmux_up=active → recent(<7d)=idle → closed. serde preserve_order,
      in-proc Mutex + atomic rename + mkdir file lock, never clobbers a corrupt file.
      Commands set_lifecycle/set_pin/set_note/touch_place. UI: grouped nav, search, pin,
      lifecycle controls, note, touch-on-open. bats 118/118 (bash 3.2 + 5.3), cargo/tsc/vite clean.

---

# Part 3 — Expanded scope (new requirements, 2026-07-20)

David added four requirements that reshape the product from a single-repo viewer into a
multi-project workbench, and raise the CLI's architecture as an open decision.

## New requirements
1. **Create worktrees from the UI** (was implied, not built). A "new place" form →
   `worktrees new <branch> [base] [--name]`. Also `switch`, `rm`, `open` as UI actions.
2. **Multiple + nested projects open at once.** Not limited to one folder tree — the UI
   tracks N repos simultaneously and supports nested projects (a git repo inside another,
   or sub-projects of a monorepo). Nav groups by PROJECT, then lifecycle within.
3. **Open directories** — a native folder picker to add a project to the workbench.
4. **CLI: bash script → full application (DECISION).** Consider rewriting the CLI as a real
   binary; the bash script becomes a thin shim that calls it. Big identity implication.

## Revised roadmap
- [ ] Phase 11 — **P2.1 UI mutations**: `new_place` (+ switch/rm/open) commands + forms.
      (Depends only on the CLI, which already does all of this — thin `CmdResult` wrappers.)
- [ ] Phase 12 — **P-Multi**: multi-repo workbench. Global tracked-projects list
      (`~/.config/worktrees-ui/projects.json`), native folder picker
      (tauri-plugin-dialog), nav grouped by project, aggregate places across repos.
      Nested-project handling (define semantics — see open questions).
- [ ] Phase 13 — **P3 infra convention** (`.worktrees.toml`, port slots, up/down) — unchanged.
- [~] Phase 14 — **P-CLI-app**: migrate CLI bash → Rust `worktrees-core` (see MIGRATION.md).
      Design DONE (workflow wf_fef6ff0a-ba3, 6 increments). Building:
      - [x] Inc 0 — workspace scaffold + core primitives — DONE (PR #4, branch feat/rust-core).
            Cargo workspace (core+cli+app joined); model/config/sysclock/git/tmux/error; CLI
            version/help; rust CI job. cargo build --workspace green, core tests 6/6, bash
            untouched. No behavior change.
      - [x] Inc 1 — read-path `ls`/`ls --json` — DONE (PR #5, branch feat/rust-core-ls).
            Project::discover + render.rs table + live-only ls_json; bin/worktrees-rs shim;
            make test-rust + CI conformance job. ls.bats 12/12 + json.bats 13/13 + misc plumbing
            4/4 vs the binary; golden bash-vs-Rust byte-diff of ls + ls --json IDENTICAL. The
            biggest risk (human-table byte-parity) is RETIRED.
      - [x] Inc 2 — write ops (new/co/switch/open/rm) — DONE (PR #6, branch feat/rust-core-ops).
            ops.rs (DWIM/guards/prompts), ui.rs (Ui trait + CliUi), tmux.rs launch + sq +
            exact-session kill; hand-rolled arg parsing. FULL parity: 118/118 unit + 3/3 real-tmux
            vs the binary. Bash untouched (118), lint clean, core 6/6. CI rust job runs whole suite.
      - [x] Inc 3 — flip default CLI to the Rust binary + distribution — DONE (PR #7 → 640b926).
            bash→bin/worktrees.bash (parallel gate); bin/worktrees = shim; Makefile test/test-bash/
            install retargeted; CI jobs lint/rust/test(Rust)/bash(engine+3.2)/install; release.yml
            cross-compiles aarch64/x86_64 × macOS/Linux + checksums; install.sh downloads prebuilt
            (portable sha256) or builds from source. Store-present bats case. All CI green both OSes.
            (store.rs→core DEFERRED to Inc 4, where the app switches to core — single move, no dup.)
            IMPORTANT FIX: common.bash set WT_BIN unconditionally → Inc 1/2 `make test-rust`/CI
            conformance silently ran BASH, not the binary. The Rust binary was FIRST genuinely
            bats-gated by Inc 3's `test` job (green). Fixed common.bash to honor a WT_BIN override;
            now `make test`=Rust and `make test-bash`=bash both real (119 each). Adversarial review
            (subagent) caught 2 release landmines pre-merge (version gate + shim-as-artifact) — fixed.
      - [x] Inc 4 — app consumes core as a LIBRARY — DONE (PR #8 → 39f9cad). store.rs moved into
            core (canonicalized repo path); Project::ls()->LsJson typed; app list_places via
            Project::discover().ls() + core::store overlay + reconcile; set_*/touch_place →
            core::store; dropped worktrees_bin/cli_ls_json/WORKTREES_BIN; PTY unchanged. cargo
            build --workspace green; core 6/6; both CLI gates 119. (App crate not yet CI-gated —
            needs webkit; that's P4 app-CI.)
      - [ ] Inc 5 — multi-project tree (open N projects) + folder picker + create-worktree-from-UI
            (David's asks) ; retire bash engine + bash CI (after a few green binary releases).
      (P2.1 UI mutations + P-Multi + P3 infra now land IN the Rust core, folded into Inc 2/5/later.)
- [ ] Phase 15 — **P4 polish/packaging** + app CI (Rust + frontend jobs).

## DECISIONS MADE (2026-07-20)
- **CLI-as-app = YES, Rust `worktrees-core` crate** shared by a `worktrees` CLI binary AND the
  Tauri app. Bash `bin/worktrees` becomes a thin shim that execs the binary. Do this FIRST
  (Phase 14 moves ahead of P-Multi/infra) so later features are built once in the shared core.
- **"Nested" = a nav TREE**: each open project (a dir/repo) is a top-level node; its worktrees
  are nested children. Support MULTIPLE projects open at once. NOT submodules / monorepo internals.
- **Build order now**: (14) design + start the Rust-core migration → (11) P2.1 UI mutations →
  (12) P-Multi project tree + folder picker → (13) P3 infra → (15) P4 polish/packaging + CI.
  P2.1/P-Multi/infra get implemented in the Rust core, not bash.

## Decisions needed (flag before building the relevant phase)
- **CLI-as-app language & shape.** Options: (a) keep bash (thin, zero-dep, works anywhere —
  current identity); (b) Rust binary that SHARES a `worktrees-core` crate with the Tauri app
  (one source of truth, no bash JSON pain, typed, but loses single-file/curl-install/bash-3.2
  story); (c) Go binary. Recommendation leaning: **(b) Rust core crate** — the app is already
  Rust, so CLI + app share logic; keep a bash `worktrees` shim that execs the binary for
  muscle-memory + non-GUI use. This is fork-eval-grade; treat as its own evaluation.
- **"Nested projects" semantics.** (i) independent repos that happen to be nested on disk;
  (ii) git submodules; (iii) sub-packages of one monorepo. Changes discovery + nav model.
- **Does the CLI rewrite happen BEFORE or AFTER P-Multi/infra?** If we go Rust-core, doing it
  earlier means P-Multi/infra are built once (in Rust) not twice.

## Open questions for David
- Nested = which of the three meanings above (or all)?
- CLI-as-app: commit to it, or keep bash for now and revisit? If commit → Rust-core preferred?
- Multi-repo: one flat aggregated nav, or per-project collapsible sections?

## App layout decision
- App lives in `app/` at repo root (Rust backend `app/src-tauri`, React+TS frontend `app/src`).
  CLI stays at `bin/`. Windows = non-goal (no tmux).

## Build decisions (from DESIGN.md)
- Stack Tauri v2 (Rust core + React/TS). macOS+Linux only (Windows non-goal — no tmux).
- Terminal = PTY child `tmux attach-session` (detach on close, never kill).
- Declared store `$MAIN_ROOT/.worktrees.places.json` (Rust sole writer, mkdir O_EXCL lock,
  atomic rename); CLI read-only, folds it + emits `lifecycle_effective`.
- Infra config `.worktrees.toml` (no inline tables); auto-detect must reproduce ALL 7 CDV ports.
- First PR = CLI `ls --json` only (additive, keeps 104 bats green). Highest-severity item:
  C0-control-safe JSON escaper.
