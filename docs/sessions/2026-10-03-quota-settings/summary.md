# quota-settings — the user decides when a nearly spent plan stops a launch

- **Date:** 2026-10-03
- **Place:** `quota-settings` (branch `quota-settings`)
- **PR:** [#437](https://github.com/penard-monkey/worktrees/pull/437), squash-merged as `dff1052` after one review round
- **Release:** none (under `## [Unreleased]`)
- **Planning files:** `planning.tar.gz` beside this file (`task_plan.md`, `findings.md` with the design, `progress.md`)

## What shipped

- **`[quota]` in `~/.config/worktrees/config.toml`** (`crates/worktrees-core/src/quota.rs`):
  - `gate = false` never refuses a launch on usage. It returns before the usage probe, so it costs no keychain read, no GET and no `codex app-server` spawn.
  - `weekly_warn_pct = N` (1–100): a WEEKLY window refuses at `>= N`%, whatever the provider grades it. Every other window, i.e. the 5h one, keeps the provider's grade.
  - Absent keys are exactly the #358 behaviour.
  - `Policy` / `UserPolicy` / `policy_from` / `user_policy`. A bad `gate` is ON and a bad % is the provider's grade, and both are reported as `problems`, never silently.
  - `with_policy` / `set_user_policy[_at]`:
    - edits the two values in place, keeping inline comments and the line ending (CRLF stays CRLF);
    - re-parses the result and refuses layouts it cannot edit (`quota.gate = …`, `quota = {…}`).
- **`Window.weekly`**, decided from the reader's STRUCTURE:
  - Claude: `UsageLimit::is_weekly` = `kind.starts_with("weekly")` (`claude_usage.rs`).
  - Codex: `Limit::is_weekly` = `window_minutes > 24h` (`codex_usage.rs`).
  - The fixture seam reads `"weekly": true`.
- **The refusal** names the rule that fired ("the user's limit: 90% of a weekly window", or "Claude graded it nearly spent") and the remedy. The remedy is `how_to_change(&config_toml_path())`, which uses `SETTINGS_PATH` ("Settings → Behavior → Plan limits") and names the file actually read (XDG honoured, `~`-abbreviated).
- **`gate()` → `gate_with(adapter, launch, now, policy, how)`**: the `_with` seam the gate-off test drives with a panicking `usage()`.
- **`quota` in `projcfg::USER_ONLY_KEYS`**: a repo's `.worktrees.toml` setting it is a hard error.
- **`config::edit_user_config(path, edit)`**: the ONE writer for `config.toml`, now used by `quota`, `reach` (`cross_project`) and `trust` (pi). It does three things:
  - resolves a symlink and renames onto the TARGET, in the target's own dir;
  - copies the original mode onto the temp file;
  - holds a process-wide lock across the read-modify-write.
  Test helper: `config::assert_writes_through_a_link`.
- **App:**
  - `quota_settings` / `set_quota_settings` (`app/src-tauri/src/lib.rs`).
  - `QuotaSection` (`app/src/QuotaPanel.tsx`) in Settings → Behavior → Plan limits: a toggle, and a % select disabled while the gate is off.
  - The problems line is in `--txt-dim` with a `--warn` glyph.
  - `.setting select:disabled` is dimmed (`App.css`).
  - The mock tracks both commands (`?quota=off`, `?quotapct=<n>|bad`).
- **`app/scripts/quota-check.mjs`** pins three things:
  - the offered %s against `PCT_MIN..=PCT_MAX`;
  - `SETTINGS_PATH` against where `<QuotaSection>` actually renders;
  - both commands: defined, registered, mocked.
- **bats** (`test/quota.bats`): 9 `quota policy:` cases:
  - gate off;
  - 85% / 92% at 90;
  - 5h under a weekly threshold;
  - a nonsense %;
  - the remedy followed end to end;
  - MCP `create_worktree`;
  - a repo's `.worktrees.toml`;
  - the XDG path.
- README section on `[quota]`; CHANGELOG `### Added` (the knobs) and `### Fixed` (the symlink/mode writer).

## Decisions

- **One knob, weekly only.** David asked about weekly. The 5h window is the one a parallel spawn burns through, so it keeps the provider's grade, and the escape from it is gate off. Two knobs (5h and weekly) would have doubled the surface for a case nobody asked for.
- **A threshold overrides the grade in BOTH directions.**
  - 85% graded `warning` at N=90 launches; 92% graded `normal` refuses.
  - Rejected: using it only as a higher bar on top of the grade. It would have made "warn at 80%" impossible for a window the provider calls normal.
- **"Weekly" is never a label match.** Codex labels a bucket by its NAME (`Codex`), not its span, so `"7d"` matching would have failed silently on every Codex window. A window of unreported length is not weekly, which means today's behaviour.
- **Read at gate time, not at server start**, unlike `cross_project`. A change reaches the app and every long-lived `worktrees mcp` on the next launch with no restart, which matters because those servers outlive everything.
- **Lenient but loud on bad values.** A typo must neither lock the user out of launching nor silently turn the gate off. So a bad `gate` keeps the gate on, and Settings shows the problem.
- **The usage meter's tint is NOT governed.** It is a gauge of the provider's reading; turning the gate off must not make it lie. This is said in the panel's hint.
- **The panel lives in Behavior**, not in Claude, Codex or Agent guidance. It governs launches for every harness that reports usage, and Agent guidance is about what agents are told.
- **The panel is self-loading** (it invokes its own status on mount) rather than threaded through App like `CrossProjectSection`. No offer or nav state depends on it.

## Dead ends / gotchas

- **The Bash tool's `grep` is a shell function here** (from the zsh snapshot), and it silently returned NOTHING for patterns that were plainly in the file (`USER_ONLY_KEYS` in `projcfg.rs`). That looked like "the machinery the brief names does not exist". `/usr/bin/grep` found it at once. Use the absolute path when a search comes back empty and should not.
- **`git checkout <file>` to undo a mutation also undid the real, uncommitted edit** in that file (SettingsSheet's import and `<QuotaSection>`). It was only noticed because `git diff --stat` lost a line. Restore a mutation from a backup copy (`cp file.bak file`), as every other mutation in this session did.
- **The reviewer's must-fix was not about this PR's logic.** `reach` and `trust` had had the same tmp-and-rename writer for weeks:
  - a stow/chezmoi `config.toml` became a regular file, and the dotfiles copy kept the OLD content;
  - a 0600 file came back 0644.
  Copying an existing writer's pattern copied its bug. The fix went into a shared helper rather than a third copy.
- **The mode half had to be shown red separately.** With all three writers replacing the link, the test died on the link assertion before reaching the mode check. So the helper also checks a plain 0600 file, and the mode fix was shown red with link-following in and `set_permissions` removed.
- **The XDG test uses a `_with` seam, not `XDG_CONFIG_HOME`.** Setting a process-global env in a unit test races every parallel test that reads the user config. `gate_with` takes the policy as an argument; bats covers the real env path.
- **Keeping an inline comment needed the whole whitespace run before `#`**, not just the last space (`find(" #")`), or `gate = true  # x` lost one space of alignment.
- **The installed CLI was 0.36.1, not 0.37.0**, so the bats fail-first ran against 0.36.1. Fine for this purpose, but say which binary.
- Harness: `vite` started inside a tool call dies (143) when killed between restarts, as AGENTS.md says. The first Settings click after a reload sometimes landed before the button existed, and only a retry opened it.

## Verification

- **Fail-first, each shown red then green:**
  - core:
    - the threshold test, before `trips` was wired in;
    - mutations: no gate check, threshold on every window, label-based weekly (Claude and Codex), `quota` not user-only, a setter that keeps old keys;
    - review round: link (×3 writers), mode (×3), no early return before the probe, always-LF, `starts_with("[quota")`, a hard-coded path.
  - bats: all 8 first-round cases failed on the shipped 0.36.1 binary; the XDG case failed on a mutated rebuild.
  - `quota-check.mjs`: an out-of-range %, a renamed heading, the section moved out of Behavior, and `SETTINGS_PATH` naming "General".
- **Gates at the merged tip** (`1947ad5`, before the squash):
  - `make test` 1..472 with 0 not ok; lint clean;
  - `cargo test`: core 706 passed (1 ignored), cli 70, app --lib 154;
  - `tsc`, `cargo check -p app`, `test-frontend` 33 ok;
  - `ls --json` byte-identical to `~/.local/bin/worktrees`;
  - CI green 9/9 on both runs.
- **Mock harness (Chrome):**
  - render, problem line, 90% write, gate off disabling and dimming the select, initial off state;
  - problems-line contrast across all six themes, composited: 3.15–7.89:1, against 2.24–4.02:1 for `.hint`.
- The reviewer separately checked the panel in WebKit.
- **Not done:** the real app was not run, and no real spent window has been seen (see ROADMAP's standing item from #358).

## Follow-ups (in ROADMAP)

- Codex refusals read "at 81% of its Codex window" because the window's label is the bucket's name.
- Whether `weekly_warn_pct = 100` should still allow a window the provider grades `over` at 99.9%.
