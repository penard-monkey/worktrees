---
title: "Session — the wheel scrolls a Codex pane"
---

# Session — the wheel scrolls a Codex pane

- **Date:** 2026-10-02
- **Worktree:** `.worktrees/codex-scroll`
- **Branch:** `codex-scroll`
- **PR:** [#422](https://github.com/penard-monkey/worktrees/pull/422), squashed as `94bf1af`. It is two commits: the fix, then the review fixes.
- **Release tag:** none yet; it is under `[Unreleased]`.
- **Planning files:** `planning.tar.gz`. It holds the lane's brief, `.planning/brief.md`. There are no task_plan, findings or progress files.

## What shipped

**Root cause.** The live gif-stickers Codex pane (codex-cli 0.159.0) measured:
- `alternate_on=1`;
- every `mouse_*_flag=0`;
- `history_size=0`.

Codex draws its TUI on the alternate screen and never asks for the mouse, so tmux keeps no history. The #373 `wheel_plan` could only send `Up`/`Down` for such a pane, and Codex reads those as composer history.

**Fix.** Codex is launched inline with `--no-alt-screen`. Its output then lands on the pane's main screen and in tmux history. The existing main-screen wheel path (`copy-mode -e` + `scroll-up`, the pi path) scrolls it, so the app needed no change.
- `crates/worktrees-core/src/harness.rs`:
  - `Codex::launch_args` pushes `CODEX_INLINE` into `head`, after the two `-c` words and before the place flags. That puts it before a `resume --last` subcommand, so fresh launches and resumes both get it.
  - It is skipped when a whitespace-split word of the user's own command is already the flag.
- Tests:
  - `codex_runs_inline_on_launch_and_resume`;
  - `an_inline_codex_still_shows_its_modal_footer` (in `codex.rs`), over four real captures in `crates/worktrees-core/tests/fixtures/codex-inline/`: an approval and a plan-mode question, each live and cancelled;
  - two exact-argv pins were updated (`launch_args_split_head_and_tail`, `launch_cmd_names_claude_and_briefs_both_agents`).
- Docs:
  - `AGENTS.md` has a note in the Codex rollout section;
  - the wheel item in `docs/adding-a-harness.md` is sharpened into "the wheel scrolls it", with what to measure;
  - `CHANGELOG.md` has one `### Fixed` entry, which states the version floor.

## Decisions

- **Inline launch, not a Codex branch in `wheel_plan`.** Alt-screen Codex does scroll its own transcript on PageUp/PageDown, but only a page per key. That puts it in a "↓ Back to bottom · esc" mode, and the composer hint changes to "enter/esc latest". S-Up, C-Up and M-Up do nothing, and C-p recalls global prompt history. Mapping wheel notches onto pages would be coarse and Codex-specific. Inline is line-granular and reuses a path that is already tested.
- **Per-launch argv, never `~/.codex/config.toml`.** This is the same rule as the `-c` words already in the head.
- **The duplicate guard matches a whole word.** Passing the flag twice is a hard clap error (`codex --no-alt-screen --no-alt-screen` exits 2 with "cannot be used multiple times"), so the guard is needed. A substring match would suppress the flag for `-c x=--no-alt-screen-ish`. The test has that case, and it goes red under a `contains()` mutation.
- **Version floor: codex-cli ≥ 0.81.0.** The orchestrator's review found the flag arrived in openai/codex #8555 (2026-01-09) and first shipped in rust-v0.81.0. An older Codex rejects the flag, and the lane falls back to the shell. Nothing in the repo enforces a minimum Codex version; the floor is documented, not checked.
- **Lanes that are already running are not migrated.** They keep the alternate screen until they are relaunched.

## Dead ends / gotchas

- **Enter in a fresh probe can answer Codex's "Trust this folder?" prompt, and Codex SAVES that answer.** I sent `2` and then `Enter` to dismiss the update prompt. The second screen was the folder-trust modal, and Enter chose "Trust and continue". Codex then wrote `[projects."…/codex-scroll/repo"] trust_level = "trusted"` into `~/.codex/config.toml`. The brief forbids writes there, so it was left for David to remove (ROADMAP). Dismiss probe prompts with Esc only, and capture the screen before every keypress.
- **zsh does not word-split `$var`**, so `set -- $sz` in the Bash tool left `-y` with no argument. Put tmux probes that loop over sizes in a `#!/bin/bash` file. This is the same family as the zsh `=word` note in the pi-scroll session.
- **Inline Codex pushes its startup modals into history** (the update prompt, the trust prompt, an early banner). This is only cosmetic, and the first resize clears it, because Codex clears and re-emits its whole transcript on resize.
- **`codex -a untrusted` is not a value on 0.159** (only `on-request` and `never`). The approval probe used `-a on-request -s read-only` and `touch b.txt`.
- **`resume --last` picks the newest session in the cwd**, including one that is still open. That one shows "This conversation is open in another app", but the transcript is replayed into history either way.

## Verification

All on a throwaway server: `tmux -L codexscroll-probe -f /dev/null`, through an `env -u TMUX` wrapper, killed afterwards. The pane was 100×30.

| | alt-screen | `--no-alt-screen` |
|---|---|---|
| `alternate_on` / `history_size` after an 80-line reply | 1 / 0 | 0 / 122 |
| `copy-mode -e` + `scroll-up 15` | (no history) | earlier lines visible |
| Resize 70×25 → 120×40 → 60×20 → 100×30 | — | history 122 → 80. The transcript is re-emitted: the prompt once, `80` once, stale startup frames dropped. |
| `resume --last` | — | the transcript is replayed into history (104) |
| Approval modal | — | the bottom line is `Press enter to confirm or esc to cancel`. After Esc, the footer is on neither the screen nor in history. |
| Plan-mode question | — | the bottom line is `… enter to submit answer \| esc to interrupt` |

Fail-first and mutations:
- `codex_runs_inline_on_launch_and_resume` was red before the fix (`harness.rs:881`). After review it is also red under a `contains()` guard (`:904`).
- `an_inline_codex_still_shows_its_modal_footer` is red under a mutated `APPROVAL_FOOTER`. It pins existing behaviour, so a pre-fix run proves nothing.

Gates:
- release build fresh;
- bats `1..451`, 0 not ok; lint 0;
- core 667 passed (1 ignored); cli 70; app --lib 151;
- tsc and `cargo check -p app` clean; `make test-frontend` 30 ok;
- `ls --json` byte-identical to the installed 0.36.0.

CI 9/9 on both pushes (runs 37077797604 and 37079463317). The frontend gates ran on node 26 with pnpm 11.5.2, because the pinned node 22.23 is not installed.

**Not done:** the real app. David's running app was off limits.

## Follow-ups

In ROADMAP:
- David's hand test of a fresh Codex lane;
- whether the flag beats a user's `tui.alternate_screen` config;
- the stray `trust_level` entry in `~/.codex/config.toml`.
