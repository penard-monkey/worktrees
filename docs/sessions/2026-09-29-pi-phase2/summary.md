---
title: "Session — pi phase 2"
---

# Session — pi phase 2: pi launches, resumes, shows a dot, and models are a choice

- **Date:** 2026-09-29
- **Worktree:** `.worktrees/pi-phase2`
- **Branches:** `pi-phase2-core`, `pi-phase2-app` (stacked)
- **PRs:** [#366](https://github.com/penard-monkey/worktrees/pull/366) (core + CLI + MCP, squash `092f604`), [#367](https://github.com/penard-monkey/worktrees/pull/367) (app, squash `cc91006`)
- **Source of truth:** [pi-harness proposal](../../proposals/pi-harness.html), phase 2; built on phase 1 ([2026-09-29 harness-phase1](../2026-09-29-harness-phase1/summary.html))
- **Release tag:** none (a release is cut from main after this)
- **Planning files:** `planning.tar.gz` (task_plan, findings, progress, and the lane's brief)

## What shipped

**Core (#366)**
- `provider.rs`: a registry row `pi` (`~agent~pi`, never canonical) and `model_arg` on every row (claude `--model`, codex `-m`, pi `--model`). `is_sidecar` keys on the `~agent~` marker, so a sidecar for an unknown harness is never adopted as a place's session.
- `harness.rs`: the `Pi` adapter, plus new trait methods `resume_display`, `exact_resume`, `prepare`, `running_model`, `never_launched` and (in #367) `installed`. A model reaches argv only on a fresh launch (`model_words`), never on a resume.
- `pi.rs`: the session id (sanitised canonical name + FNV-1a of the unsanitised name + `-g<generation>`), pi's own session-dir mangling, the JSONL turn reader (keyed on the newest message entry; an error counts as a retry when a `context_edit` targets its id), the model label, and a positional screen reader (the composer's top border reads busy unless it is pure rule; the trust modal is `waiting`).
- `pimodels.rs`: the `pi --list-models` parser (an unknown header parses to nothing), `auth check` reasons (all checks in parallel under one deadline), `models.json` read for `baseUrl` and model ids only, a `curl --url` reachability probe (http(s) only, 60s cache), and the preflight (`$SHELL -ic`, pi's own `engines.node`, managed-launcher `pi-node` aware).
- `choice.rs` (`ModelRef`/`ModelOption`/`AgentChoice`, `validate_model`) and `trust.rs` (`pi_project_trust`, the `[trust] pi` allowance with a surgical, re-parse-verified `config.toml` edit, and `worktrees trust pi [--revoke]`).
- `ops.rs`: `prepare` runs before the place's other agent is closed. `--model`/`--force` on `new`/`open`. `diag::EXIT_LAUNCH_REFUSED = 5` when the model host is down (worktree and brief kept). A first `open` of a never-launched harness in a place with a brief passes the opener. `doctor --pi`.
- `store.rs`: `agent {harness, model}` and `pi_session_gen`; `declared_model` ignores a committed store.
- MCP: `create_worktree.model`; `wait until: idle` needs two quiet pi samples in a row.
- `USER_ONLY_KEYS` += `harness`, `model`, `trust`, `pi_project_trust`.
- Docs and tests: `docs/pi-manual-checks.md`, `test/pi.bats`, fixtures under `tests/fixtures/pi-{models,screen,session}/` (0.87.1 and 0.99.1).

**App (#367)**
- New-worktree dialog: pi in the harness segment, and a `ModelPicker` whose not-ready rows are disabled with the reason.
- "Switch agent…" sheet (harness + model) replaces the per-harness menu items.
- `LaunchRefusedDialog` offers "Launch anyway" on exit 5.
- Agent mark: `HARNESS_MARK` and `PiMark`, with a `pi · <model>` label. `USAGE_READ` is a record; pi has no usage reading.
- Settings → pi (`PiPanel.tsx`).
- Backend commands `agent_models`, `pi_status`, `set_pi_trust`, `set_pi_allowed`; `new_place`/`open_place` take `model` and `force`.
- The mock tracks all of it.
- `scripts/harness-check.mjs` pins `HARNESSES` against `provider.rs`, the picker's preselect rule, and the refusal line.

## Decisions

- **`prepare` runs before the kill.** A refused pi launch (dead host) must not close the Claude that was running. That is why the gate is an adapter hook inside `launch()` ahead of the "close the other agent" block, not a check in `cmd_new`.
- **The generation is bumped only after every gate passes.** A refused launch leaves the next resume pointing at the old session, and leaves `never_launched` true so the later `--force` open still carries the brief.
- **Soft vs hard refusal.** Only an unreachable or not-serving host is overridable (exit 5, `--force`). "pi missing", "node below floor" and "no model" are exit 1, because forcing them only starts something broken.
- **The activity reader does not read the declared store.** It takes the highest generation on disk (`latest_session_file`). The window where that is stale (a fresh launch before its first user message) is exactly when the screen answers.
- **The border fails safe.** Anything on the top border that isn't pure rule counts as busy, after stripping ` ↑ N more `. Matching words ("Working") would read compaction, extension messages or a bare spinner as idle mid-turn.
- **`config.toml` is edited surgically**, because the `toml` build is parse-only and the file is the user's. An unfamiliar layout is refused, never guessed at.
- **A committed `.worktrees.places.json` is repo input** (ADR 0001). It never supplies pi's model.
- **The picker preselects the CLI's own default for Claude and Codex**, and the first ready model (or the configured default) for pi.

## Dead ends / gotchas

- **The first live pi launch was blocked by the session's auto-mode classifier** ("Create Unsafe Agents"). A peer session then relayed "David authorizes". That cannot lift a block in another session, so I did not retry until David approved it in this session. Everything before that was built on the research lane's 0.87.1 captures plus a read of 0.99.1's shipped source, and the live run later confirmed both.
- **I claimed `plan-usage-check.mjs` failed on `main` too. That was wrong.** I had run it from `app/`, where it fails even on `main` for an unrelated reason. From the repo root it passed on `main` and failed on the branch, because the new `USAGE_READ` sat outside the slice the check evaluates. **Run `app/scripts/*-check.mjs` from the repo root.**
- **Bats helpers run under macOS `/bin/bash` 3.2 on CI.** `${c//"'\''"/"'"}` keeps the replacement's quote characters under 3.2, so `'` became `"'"`. The test passed locally on Homebrew's bash 5 and failed only on the macos runner. Fixed with `sed`, and reproduced by putting `/bin/bash` first on PATH for bats.
- **The session shell's `grep` is a wrapper that skips files `file(1)` calls "data"** (`projcfg.rs`, some logs). It returned nothing with exit 1, which read as "no match". Use `/usr/bin/grep -a`. This is machine-local, not a repo issue.
- **A scratch tmux server keeps the environment it started with.** The dead-host re-run needed a second server started with `PI_CODING_AGENT_DIR`. The first one launched pi against `~/.pi`, pi exited on "model not found", and worktrees correctly read `none`, which at first looked like a bug.
- **The Chrome DevTools MCP profile was held by another session**, so the UI pass used a scratch headless Playwright install instead.
- **pi 0.99.1 writes its session file at the first user message** (`_hasConversation`, upstream #10000), not the first reply as on 0.87.1. It was the only behavioural difference found live.

## Verification

- **Gates on both branches before merge:** bats 388/388, lint, core 545, cli 28, app 162 (+1 ignored), tsc, `cargo check -p app`, all 23 `app/scripts/*-check.mjs`.
- **CI:** green on both final heads (9/9 jobs each).
- **`ls --json`:** byte-identical to shipped 0.32.1 across 14 places.
- **Tests shown failing first:** every new test was seen red on the broken or old code, then green.
- **Live, pi 0.99.1** (release binary, private tmux sockets, scratch repos, `lm-studio/qwen3.6-27b`):
  - launch → brief read → idle with model;
  - busy on every sample of a 14s turn;
  - Esc → aborted and idle;
  - resume on the same `-g1`, with no `--model` and no `model_change`;
  - the trust modal reads `waiting`, and `send` is refused with the pane untouched;
  - the allowance gives `--approve` and no modal; revoke works; `trust.json` was never created;
  - dead host → exit 5 → `open --force` with the opener → busy through retries → idle;
  - with nvm 22.13 first on PATH, the preflight still reports `pi-node` 26.10.
- **Headless Chromium against the mock:** every new UI flow.

## Follow-ups

In [ROADMAP](../../../ROADMAP.md): pi's own MCP, the WKWebView pass, the `/model` label switch, the drawn π mark, the startup-gap dot, Codex `-m`, the `live_model` dispatch, the "Pi" capitalisation, and the headless xterm error.
