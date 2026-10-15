---
title: "ADR 0002 — the worktrees rules may be written into a repository, as data"
---

# ADR 0002 — the worktrees rules may be written into a repository, as data

- **Status:** accepted
- **Date:** 2026-10-14
- **Supersedes:** `docs/proposals/agent-guidance.md` §4.4, first bullet
  ("Not recommended: writing guidance into repos").

## Decision

`worktrees rules apply` writes the worktrees way of working INTO a repository:
a managed block at the end of the root AGENTS.md, the `close-out` skill at
`.claude/skills/close-out/SKILL.md`, and a one-time `.claude/close-out.md`. It
is opt-in, per repository, and committed on the local `agent-instructions`
branch for the user to review and push.

The per-launch guidance (`guidance.rs`: MCP instructions, the `worktrees`
skill per launch) is unchanged and stays the newest word. The repo copy is the
FLOOR for sessions worktrees did not launch — a hand-started `claude`, a
teammate — and says that the skill wins where they differ.

## Why the reversal

§4.4 rejected repo guidance for four reasons. Each now has an answer:

| §4.4 said | Now |
|---|---|
| per repo | It is meant to be: the rules are part of how THAT repo is worked on, and teammates without worktrees' launch path read them. |
| churn in every repo's history | Opt-in, one commit per version the user chooses to apply. |
| stale the day the tool changes | A versioned, hashed block: `rules status` says `outdated`, `rules apply` replaces it in place, and a hand edit is detected and left alone instead of overwritten. |
| does not reach untrusted pi | Neither does anything else in the repo; per-launch delivery still covers pi. |

And one thing §4.4 did not weigh: per-launch delivery reaches only sessions
worktrees starts. The ask (David, 2026-10-14) was that "spin up a lane" works
in any session in a managed repo without the agent relearning it.

## Boundaries — what keeps ADR 0001 intact

- The repo copy is **data worktrees writes**. Reading it back parses two
  markers and hashes the body; nothing in it becomes argv, a launch prompt, or
  per-launch delivery — `guidance::materialize` takes no repository at all
  (its inputs are the binary's text and the user's own edit under
  `~/.config/worktrees/guidance/`), so the per-launch skill cannot pick the
  repo copy up. The one thing repo data DOES steer is a verdict: the version
  and hash parsed from the markers decide what `rules status` reports and
  whether the app suggests an apply. That is a read of data into a state,
  never into a command.
- `.claude/close-out.md` lists gate commands as **text for an agent to read**,
  like AGENTS.md always has. worktrees never runs or parses them; `rules` must
  never grow a "run the gates" verb.
- The `worktrees` skill itself is NOT written into repositories: a launched
  Claude already has it from the per-launch plugin, and a second copy would be
  a second, older skill beside it.
- In AGENTS.md, nothing outside the markers is ever touched. The same commit
  may also carry the layout fix (rename CLAUDE.md, write the stub, link
  skills for Codex), the close-out skill and `.claude/close-out.md`; a
  `close-out` skill worktrees did not write, or a symlink on its path, is
  never written over or through, not even with `--force`.
- Nothing leaves the machine unless the user asks (`--push`); MCP
  `apply_rules` never pushes.

## Revisiting

Reopen if harnesses gain a channel that reaches hand-started sessions without
a repo file (then the block can shrink to a pointer), or if the repo copy and
the per-launch skill start contradicting each other in practice.
