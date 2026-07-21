# MIGRATION — bash CLI → shared Rust `worktrees-core`

> Decision (owner): rewrite the CLI as a Rust `worktrees-core` crate shared by a `worktrees`
> CLI binary AND the Tauri app; the bash `bin/worktrees` becomes a thin shim that execs the
> binary. Done BEFORE more features so multi-repo + infra are built once. Produced by the
> `design-rust-core-migration` workflow (4 facets → critique → synthesis). See DESIGN.md for
> the app, task_plan.md for status.

## Why & the load-bearing decisions
- **Shell out to `git`/`tmux`** via `std::process::Command` (always `git -C <cwd>`), NOT gix/git2.
  It's a 1:1 port of the bash invocations; it preserves the **stale-dir upward-resolution trap**
  (a data-loss-class guard) exactly; and it's the ONLY thing that keeps the bats fake-tmux/git
  PATH shims intercepting the compiled binary.
- **Conformance gate with zero harness edits:** point `WT_BIN` at the bash **shim** (a shebang
  script). Verified: `bash <compiled-binary>` fails ("cannot execute binary file") but
  `bash <shim-script>` runs — so `run_wt`'s `exec "${RUN_BASH:-bash}" "$WT_BIN"` and `wt_answer`
  work untouched; the shim execs the binary, which inherits PATH so the fake shims intercept.
- **One `Place`/`LsJson` serde type in core**, re-exported to CLI + app → single source of truth
  for the `ls --json` schema v1. All rendering (human table, glyphs, JSON) lives in the CLI,
  driven by structured data + a `Report` event stream from core (so the app ignores human strings).
- **CLI `ls --json` stays LIVE-ONLY** (`declared:null`, lifecycle ∈ {closed,active}) even when a
  store sidecar exists, so it stays byte-identical to today's `emit_place_json`. The app overlays
  declared state + reconciles (as it does now). A new bats case pins this.
- **Biggest risk = human `ls` table byte-parity** (two-pass column widths, byte-length `fit()`
  truncation, color-outside-padding, the header-vs-row GIT spacing asymmetry, and ls.bats:136-139
  absolute-offset assertions). De-risk FIRST with a golden bash-vs-Rust stdout diff on a fixture.

## Workspace layout
```
/Cargo.toml                     # [workspace] virtual manifest (members: core, cli, app)
/crates/worktrees-core/         # lib: model, config, sysclock, git, tmux, project, place,
                                #      ops/{new,switch,open,rm}, store, render, cli, error, report
/crates/worktrees-cli/          # bin `worktrees` (thin main → core::cli::run)
/app/src-tauri/                 # existing Tauri crate, now a workspace member (deps worktrees-core)
/bin/worktrees                  # bash SHIM (execs the compiled binary) — from Increment 3
/bin/worktrees.bash             # the ORIGINAL script, kept until Increment 5 (dual-engine gate)
/test/                          # bats suite UNCHANGED (helpers/common.bash not edited)
```
git/tmux via subprocess; no git2/gix/portable-pty in core (PTY stays app-only).

## Increments
- **0 — Workspace scaffold + read-only core primitives.** Root workspace (app joined); core
  skeleton: `model` (serde types), `config` (prefix/ai precedence, per-byte sanitize), `sysclock`
  (BSD/GNU stat/date probe), `git`/`tmux` Command wrappers, `error`. CLI skeleton (version/help).
  `cargo build --workspace` + `cargo test --workspace` green; app still `pnpm tauri build`s.
  No behavior change; bash CLI untouched. ← **building now**
- **1 — SMALLEST SAFE STEP: read-path `ls` + `ls --json` passing ls.bats/json.bats via the shim.**
  `Project::discover`/registration gate/`worktree_for_branch`/`default_base`/`ensure_excluded`;
  `place.rs` (registration-gated status); `Project::ls_json` (live-only); `render.rs` (the table);
  cli dispatch (ls/no-args/version/help/unknown); `bin/worktrees` shim; `make test-rust`.
  Green: ls.bats + json.bats + read-path misc.bats vs the compiled binary; byte-diff clean.
- **2 — Write ops in core: new/co/switch/open/rm** (DWIM + guards + EOF-abort prompts + exact
  argv). Full 121-case suite green against the binary (incl. macOS bash-3.2 shim path). Guard
  errors exit 1 (never clap's 2 — hand-roll subcommand arg parsing).
- **3 — Flip default CLI to the binary; store→core; release/install.sh cross-compile.** New bats
  case: `ls --json` with a populated store still emits `declared:null`. Both engines gated in CI.
- **4 — App consumes core as a library** (drop the subprocess + `WORKTREES_BIN`). list_places on
  core structs + store merge; PTY unchanged; output shape byte-equivalent.
- **5 — Multi-project tree + retire the bash engine.** `core::workspace` (add/remove/list
  projects); folder-picker; nav renders project→places tree; delete `bin/worktrees.bash` + bash CI.

## Parity checklist (must not regress)
Commands/flags/DWIM/exit-codes; `ls --json` schema v1; human table byte-alignment; stale-dir
registration gate (no git write against main); branch↔place redirect; dirty guards; BSD/GNU
stat/date; tmux session reuse + exact-session kill; prefix precedence; bash-3.2 shim safety.
