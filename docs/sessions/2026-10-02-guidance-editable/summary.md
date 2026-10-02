---
title: Editable agent guidance, with a diff after updates — 2026-10-02
---

# Editable agent guidance, with a diff after updates

- **Date:** 2026-10-02
- **Worktree:** `.worktrees/guidance-editable`
- **Branch:** `guidance-editable` (squash-merged as `be64975`)
- **PR:** [#411](https://github.com/penard-monkey/worktrees/pull/411)
- **Release:** none yet; the entries are under `## [Unreleased]`
- **Planning files:** `planning.tar.gz` (`task_plan.md` only)
- **Design:** `docs/proposals/agent-guidance.md` §11

## What shipped

- **Editable skill (core).** `crates/worktrees-core/src/guidance.rs`:
  - The edit lives at `~/.config/worktrees/guidance/SKILL.md`.
  - `SKILL.base.json` stores `{version, hash, text}` of the shipped skill the edit started from.
  - New functions: `read_edit_in`, `effective_skill_in`, `save_edit_in`, `validate_skill`, `text_hash`.
  - `materialize()` now writes the effective skill. The per-launch directory is named by a content hash, so an edit gets a new directory and running sessions keep their own.
- **Compare and merge (core).** `diff_texts` and `merge_texts` run `git diff --no-index -U1000000` and `git merge-file`. Both go through `git_isolated`, which strips the user's git config and attributes.
- **CLI.**
  - `worktrees guide` prints the effective skill.
  - `--default` prints the shipped one.
  - `--status` and `--json` gain a skill line and `skill_edit` / `skill_default` / `skill_hash` / `edit_path`.
- **App commands.** `app/src-tauri/src/lib.rs` adds `set_agent_guidance_skill`, `agent_guidance_diff` and `agent_guidance_merge`. The mock covers all three, with `?skill=edited|stale|nobase|invalid`.
- **Settings panel.** `app/src/GuidancePanel.tsx`:
  - An editor (Edit…, Save, Discard, Compare with the default).
  - Reset to default, which takes two clicks.
  - An "unusable edit" card.
  - The "default changed" band. Its compare shows three views through the existing `DiffView`, and its actions are Keep mine / Use the new default / Merge into the editor….
- **New offer.** `agent-guidance-changed` in `app/src/offers.ts`. Its fingerprint is the new default's hash.
- **Tests.** `test/guidance.bats` and `app/scripts/offers-check.mjs` were extended, and the guidance unit tests went from 13 to 23.

## Decisions

- **Only the skill is editable, not the rule.** `HEAD` is also the MCP `initialize` instructions. It has to fit one line inside Codex's 247-character cut. `guard_bin` also recognises a guard-capable CLI by `guide --rules` starting with `Managed by worktrees:`. An edit could break all three without any error.
- **The base record stores the text, not only a hash.** The binary that notices the change no longer contains the old default, and the three-way compare and merge need it.
- **`stale` compares content hashes, not `VERSION`.** Someone who forked the text has opted out of getting wording fixes silently. `VERSION` only moves when a change is worth re-asking everyone.
- **"Modified" is exact.** Saving text equal to the default deletes both files.
- **The edit is written before its base** (changed in review). A save that dies halfway then leaves the edit stale, so the offer asks again rather than retiring with nothing chosen.
- **Keep mine re-saves the same text.** That re-bases it on the new default. A choice clears `stale`, while a dismissal only records a fingerprint, so the two can't be confused.
- **When the offer is not raised:** an unusable edit is a problem, shown in Settings where it can't be silenced; and with delivery off there is nothing to offer.
- **Escape with an unsaved draft is swallowed** by a no-op `useEscape` entry. The draft also survives the section unmounting, through a module-level variable.

## Dead ends / gotchas

- **`--no-ext-diff` is not isolation.** Found in review: a global `core.attributesFile` with `* diff=x` plus a `[diff "x"] textconv` rewrote every line in the Settings diff. A failing textconv exits 128, which left the box at "diffing…" forever. `merge.conflictStyle=diff3` added a `|||||||` block to the merge.
- **`GIT_CONFIG_GLOBAL=/dev/null` is still not enough.** The DEFAULT attributes file (`$XDG_CONFIG_HOME/git/attributes`) is read with no config at all, so `* -diff` there produced "Binary files differ". `HOME` and `XDG_CONFIG_HOME` are now pointed at the scratch dir. The test shows that part is load-bearing: removing it alone turns the test red.
- **`git merge-file` treats changes on ADJACENT lines as a conflict.** My first clean-merge fixture assumed otherwise.
- **zsh does not word-split `$var`.** A `for c in "-p x" …; cargo test $c` loop reported exit 101 for all three suites while every test passed.
- **#409 landed mid-review.** The PR went DIRTY, and a conflicted PR gets no CI at all. Rebasing fixed it. The CHANGELOG resolution needed one `[Unreleased]` with `### Added` and `### Fixed` each appearing once.

## Verification

- Every new core test, the bats test and the offers-check cases were shown failing first against deliberately broken code: the stale check, the invalid fallback, the reset on equal text, `materialize` ignoring the edit, the forged base, the hostile git config, the write order, the old `offers.ts`, a constant fingerprint, and a dropped `!invalid`.
- **Headless WebKit against the mock:**
  - Every new button hit-tests with `elementFromPoint`.
  - Merge → Save retires the band, and Reset is two-click.
  - An invalid save is refused inline.
  - Escape with a dirty draft keeps Settings open and keeps the draft.
  - The offer's deep link lands on the band, flashed and hittable.
- **Gates on the final rebased tree:**

  | Gate | Result |
  | --- | --- |
  | `make test` (bats) | 451/451 |
  | `make lint` | ok |
  | `cargo test -p worktrees-core` | 664 passed |
  | `cargo test -p worktrees-cli` | 70 passed |
  | `cargo test -p app --lib` | 151 passed |
  | `tsc --noEmit` | clean |
  | `make test-frontend` | ok |
  | CI | 9/9 jobs |

## Follow-ups

- **Never run in the real app.** No sandbox pass has been done. It is listed in `ROADMAP.md`.
- **The rule could be made editable** with the same store. It was deliberately not done, for the reasons above.
