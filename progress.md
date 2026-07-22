# Progress Log

## Session 2026-07-20
- Invoked planning-with-files + ultracode. Task: evaluate forking amux/emdash/coder/mux
  vs. cherry-picking into our `worktrees` bash CLI.
- Phase 1 DONE: read `bin/worktrees` (~680 lines) + README. Captured principles in findings.md.
  Key metaphor confirmed: worktree = PLACE, branch = work that flows through it.
- Workflow round 1 (wf_125eb850-44f): mux fully researched + verified (AGPL, Electron+server+web,
  own agent loop, worktrees = 1 of 5 runtimes → NOT a fork base). amux/emdash/coder research agents
  hit TRANSIENT API rate-limiting and failed; synthesis extrapolated their rows (unverified).
- Workflow round 2 (wf_81e18f58-c77): amux/emdash/coder researched + adversarially verified
  (high confidence). (Note: args.mux injection didn't take, so round-2 synthesis saw only 3
  tools — but all 4 are individually verified, so I synthesized the final 4-way comparison by hand.)
- DONE. Verdict: DON'T FORK — cherry-pick. Full matrix + 7 stealable ideas + risks in findings.md.
  Both independent synthesis runs agreed: dont-fork-cherry-pick.

## Session 2026-07-20 (cont.) — DIRECTION CHANGE: build a Tauri UI app
- David wants his OWN place-manager UI (flavor of amux/emdash), CLI kept as engine. Not a fork.
- Read CDV ancestor `~/workspace/casadelvalle/.../scripts/worktrees.sh` → revealed STACK_MODE
  infra layer (port slots + env links + docker compose project) that public repo dropped.
- Decided: stack = **Tauri**. Architecture rules locked (CLI engine + --json, state split no-DB,
  attach-tmux never own-PTY, generalize infra convention). Lifecycle states defined.
- Phase 6 DONE: design workflow `design-worktrees-ui` (wf_dc55b208-f19) — 9 agents, 692K tokens.
  All 4 facets designed + adversarially critiqued (feasibility HIGH), synthesized into one spec.
  All 5 principles upheld. Written to DESIGN.md (arch + CLI JSON schema + declared store +
  lifecycle state machine + infra convention + Tauri/tmux embedding + P0–P4 plan + first PR).
- Biggest risk flagged: tmux-in-Tauri embedded terminal → de-risk in P1 right after P0 slice.
- NEXT: David's go/no-go on building P0 (first PR = `worktrees ls --json`, CLI-only, additive).
- BUILT P0 → PR #1 (branch feat/ls-json): https://github.com/penard-monkey/worktrees/pull/1
  - bin/worktrees: json_str/json_bool/json_nn (C0-safe escaper), wt_upstream/wt_ahead_behind,
    claude_dir_for/claude_has_session, emit_place_json/emit_ls_json, --json flag on cmd_ls
    (returns before human path), WORKTREES_JSON=1, dispatch passes "$@", banner updated.
  - test/json.bats (13 cases). README + CHANGELOG updated.
  - Verified on real bash 3.2: full suite 117/117 (104+13), shellcheck clean, JSON validates.
  - Escaper tested against backslash/quote/tab/newline/0x01 → valid JSON.
  - Planning artifacts (task_plan/findings/progress/DESIGN.md) kept OUT of the PR (untracked).
- NEXT after merge: P1 — spike the tmux-in-Tauri embedded terminal (the biggest risk).
- MERGED PR #1 (squash → main 8ef1854). CI green all jobs. Fixed a bash-5.x backslash-escape
  portability bug ubuntu CI caught (json_str backslash step now uses sed; verified 117/117 on
  bash 3.2 AND 5.3). Branch feat/ls-json deleted; main synced.
- P1 START: toolchain check → Node 22 + pnpm 11, tmux 3.6a, Xcode CLT present; Rust MISSING.
  David installed Rust via rustup (cargo 1.97.1); tool shell needs `. ~/.cargo/env` per call.
- P1 BUILT → draft PR #2 (branch feat/ui-tauri): https://github.com/penard-monkey/worktrees/pull/2
  - Scaffolded app/ (create-tauri-app, Tauri v2 + react-ts, id net.casadelvalle.worktrees).
  - Rust src-tauri/src/lib.rs: list_places (shells `worktrees ls --json`), term_open/write/
    resize/close (portable-pty child = `tmux attach-session`; close kills CLIENT = detach,
    session survives; reader thread streams raw bytes over tauri Channel<InvokeResponseBody>).
  - Frontend: TerminalPane.tsx (xterm.js + FitAddon + Channel), App.tsx (nav + top bar), App.css.
  - Toolchain snags fixed: pnpm 11 blocked esbuild build → allowBuilds in pnpm-workspace.yaml.
  - VERIFIED: cargo check clean (tauri 2.11.5, portable-pty 0.9); tsc + vite build clean.
  - PENDING (David, GUI): run `WORKTREES_BIN="$PWD/bin/worktrees" pnpm --dir app tauri dev`,
    verify attach→type→resize→quit→`tmux attach -t worktrees-<slug>` still live. See app/README.md.
  - GOTCHA: ~/bin/worktrees on PATH = CDV ancestor (no --json) → must use WORKTREES_BIN.
- NEXT: David verifies P1 GUI. If green → P2 (declared store + lifecycle). If terminal issues
  (resize clamp / detach) → grouped-session attach or -f ignore-size (tmux 3.6a supports it).
- P1 VERIFIED by David (screenshot: live embedded tmux, reopen reattached, "active"). Merged PR #2.
- P2 BUILT → PR #3 (branch feat/lifecycle): declared store (Rust-owned) + lifecycle + UI.
  CLI ensure_excluded now also excludes .worktrees.places.json (bats 118/118 bash 3.2+5.3).
  Rust: store module (serde preserve_order, mkdir lock + atomic rename, never clobbers corrupt),
  set_lifecycle/set_pin/set_note/touch_place, list_places merges+reconciles lifecycle_effective.
  UI: grouped nav + search + pin + lifecycle buttons + note + touch-on-open. cargo/tsc/vite clean.
- NEW REQUIREMENTS from David (mid-P2) → captured in task_plan Part 3:
  (1) create worktrees from UI; (2) multiple + NESTED projects open at once (multi-repo workbench,
  not one folder tree); (3) open directories (native folder picker); (4) upgrade CLI from bash
  to a full application (bash becomes a shim calling it) — DECISION-grade.
  Revised roadmap: P2.1 UI mutations → P-Multi (multi-repo) → P3 infra → P-CLI-app (decide) → P4.
  Open questions logged (nested semantics, CLI language/timing, nav layout).
- NEXT: (a) let PR #3 CI settle + David verifies P2 GUI; (b) resolve Part 3 open questions before
  building P2.1/P-Multi; leaning Rust-core crate shared by CLI + app for the CLI-as-app decision.
- DECISIONS (David): CLI→Rust worktrees-core crate shared by CLI binary + app (bash = shim);
  do it FIRST; "nested" = nav tree (project node → its worktrees nested), multiple projects open.
- P2 MERGED (PR #3 → main 5e1c463). CI all green. main now has P0+P1+P2.
- Design workflow `design-rust-core-migration` (wf_fef6ff0a-ba3) LAUNCHED: 4 facets (core-crate /
  cli-parity / app-integration / build-dist) design→critique + synthesis. Key idea: reuse the
  118-case bats suite as a CONFORMANCE gate against the Rust binary; subprocess git/tmux (faithful
  port); app uses core as a lib (drop subprocess). Awaiting completion → then build increment 1
  (workspace + core read-path + `ls`/`ls --json` passing bats).
- Migration design DONE (6 increments in MIGRATION.md). Key: WT_BIN→bash SHIM (script execs
  binary) gates the Rust binary with ZERO harness edits; subprocess git/tmux; biggest risk =
  human ls-table byte-parity. App joins the workspace; PTY stays app-only; store moves to core.
- Inc 0 BUILT → PR #4 (branch feat/rust-core): Cargo workspace (crates/worktrees-core +
  worktrees-cli + app/src-tauri joined), core primitives (model/config/sysclock/git/tmux/error)
  with 6/6 unit tests, CLI version/help, rust CI job (core+cli, app excluded — webkit). Root
  target/ gitignored, nested app Cargo.lock removed. cargo build --workspace green; bash untouched.
- Inc 0 MERGED (PR #4 → main f9f84f7). CI incl. new rust job green.
- Inc 1 BUILT + MERGED (PR #5 → main 6a53966): read path in Rust.
  - core: Project::discover (git guards + pwd -P roots + prefix), render.rs (byte-exact ls table),
    project.ls_human/ls_json (live-only, registration-gated stale trap, recency sort).
  - cli: dispatch version/help (pre-guard) + ls/list + no-args + WORKTREES_JSON/--json + unknown.
  - bin/worktrees-rs shim (script execs binary) = bats WT_BIN → zero harness edits.
    make test-rust + CI conformance job (submodules).
  - VERIFIED: ls.bats 12/12 + json.bats 13/13 + misc plumbing 4/4 vs the binary; golden
    bash-vs-Rust byte-diff of `ls` and `ls --json` IDENTICAL. CI green both OSes. lint clean.
  - BIGGEST RISK (human-table byte-parity) RETIRED.
- Inc 2 BUILT + MERGED (PR #6 → main e60c2a5): write ops in Rust.
  - ops.rs (new/co/switch/open/rm — DWIM ladder, branch↔place redirect, guards, EOF-abort
    prompts, registration gate, do_switch, remove_one), ui.rs (Ui trait + CliUi byte-parity),
    tmux.rs launch/worktree_session/new_session/split_window/kill_session + sq(), project helpers
    (is_registered/wt_for_branch/default_base/wt_branch/wt_dirty/ensure_excluded), config resolvers.
  - Hand-rolled per-command arg parsing (guards exit 1). CLI dispatches all verbs+aliases to ops.
  - Explore agent extracted the exact bats contract (error strings/exit codes/tmux argv) — matched.
  - VERIFIED: FULL parity — 118/118 unit + 3/3 real-tmux vs the binary (FIRST run, no fixes needed).
    Bash engine untouched (118), lint clean, core 6/6. CI rust job runs whole suite + real-tmux
    (installs tmux) on both OSes — all green. The Rust CLI is a verified drop-in for bash.
- Inc 3 BUILT + MERGED (PR #7 → main 640b926): FLIP to the Rust binary + binary distribution.
  - bash → bin/worktrees.bash (parallel gate); bin/worktrees = shim; Makefile test(Rust)/test-bash/
    install(build+symlink binary) retargeted; version gate → Cargo.toml.
  - CI restructured: lint / rust (cargo) / test (bats vs Rust binary) / bash (bats vs engine +
    macOS 3.2) / install — both engines gated, all green both OSes.
  - Distribution: release.yml cross-compiles aarch64/x86_64 × macOS/Linux + checksums; install.sh
    downloads prebuilt (portable sha256) or builds from source (WORKTREES_INSTALL_FROM_SOURCE=1).
  - Adversarial-review subagent caught 2 release landmines (version gate grepped a gone var; release
    shipped the shim as the artifact) → fixed here (gate→Cargo.toml; ship real per-target binaries).
  - BIG FINDING: common.bash set WT_BIN unconditionally → Inc 1/2 `make test-rust` silently ran BASH
    (bin/worktrees was bash then). The Rust binary was FIRST truly bats-gated by Inc 3's green `test`
    job. Fixed common.bash to honor WT_BIN override; now make test=Rust, make test-bash=bash (119 each).
  - store.rs→core DEFERRED to Inc 4 (bundled with app-uses-core; avoids a duplicate store).
- Inc 4 BUILT + MERGED (PR #8 → main 39f9cad): app uses worktrees-core as a LIBRARY.
  - store.rs moved app→core (canonicalized repo path so app windows share file/lock). Project
    gained typed ls()->LsJson; ls_json serializes it (CLI byte-identical, 119 both gates).
  - app list_places = Project::discover(repo).ls() + core::store overlay + reconcile; set_lifecycle/
    set_pin/set_note/touch_place → core::store; removed worktrees_bin/cli_ls_json/WORKTREES_BIN;
    PTY host unchanged. app/README run cmd = `pnpm --dir app tauri dev` (git/tmux on PATH).
  - cargo build --workspace green (core+cli+app); core 6/6; CI all green both OSes.
  - KNOWN GAP: the app crate isn't CI-built (needs webkit) — deferred to P4 app-CI; verified locally.
- NEXT: Inc 5 (final) — multi-project TREE (open N projects, each node → its worktrees nested) +
  native folder picker (tauri-plugin-dialog) + create-worktree-from-UI (new_place → ops::cmd_new
  or a typed op) + switch/rm from UI; then retire bin/worktrees.bash + bash CI (after some green
  binary releases). This delivers David's original UI asks (multi-project + create from UI).
