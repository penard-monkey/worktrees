---
title: "Codex — manual verification checklist"
---

# Codex — manual verification checklist

Everything here depends on how a real `codex` binary behaves. None of it can run
in CI. The bats suite drives fake `git`/`tmux` shims and a fake `codex`, so the
automated gates prove that worktrees *composes the right launch and the right
`codex mcp add`*, and nothing about what Codex then does with it. This list is
the Codex twin of [`ai-profiles-manual-checks.md`](ai-profiles-manual-checks.md).

**Re-run it whenever `codex` is upgraded.** Several behaviours below are
undocumented and were found by measuring. A Codex release is exactly what would
break them silently.

```sh
codex --version              # verified against: codex-cli 0.157.1
readlink -f ~/.local/bin/codex   # the standalone installer lives under ~/.codex/packages
```

Use a SCRATCH repo for everything (e.g. `~/.cache/worktrees/worktrees/<tree>/probe`),
never a real project. Anything that pushes must point at a bare origin you made.

## 1. The launch

- [ ] `worktrees new probe --ai codex` → `tmux list-panes -t '<prefix>-probe~agent~codex' -F '#{pane_start_command}'`
      contains `-c forced_login_method=chatgpt`,
      `-c 'project_doc_fallback_filenames=["CLAUDE.md"]'` and, in auto-review,
      `--approve-for-me -c 'sandbox_workspace_write.writable_roots=["<main>/.git"]' -c sandbox_workspace_write.network_access=true`,
      all BEFORE any `resume --last`.
- [ ] The TUI starts. The only startup warning is "Running without the shared
      background server… (-c) requires embedded mode". That warning is expected:
      any `-c` causes it.
- [ ] **First launch in an untrusted repo** stops at "Trust this folder?". A
      pane launched by the app or MCP sits there silently until a person answers
      (see ROADMAP). Note whether the prompt's wording or its trigger changed.

## 2. Instructions and skills

- [ ] `codex debug prompt-input -c 'project_doc_fallback_filenames=["CLAUDE.md"]'`
      in a CLAUDE.md-only repo (run from a subdirectory) includes BOTH the root
      and the nested CLAUDE.md. Without the flag it includes neither. With
      AGENTS.md + a CLAUDE.md stub, only AGENTS.md loads. `debug prompt-input`
      never calls the model.
- [ ] Its skill roots include `~/.agents/skills` and repo `.agents/skills`, and
      NOT `.claude/skills`. A symlinked skill dir is followed.
- [ ] `worktrees agent-setup status` in a CLAUDE.md-only scratch repo reports
      claude-only. `agent-setup fix` against a bare origin with a fake `gh`
      creates `agent-instructions` (AGENTS.md real, CLAUDE.md = `@AGENTS.md`)
      and leaves `main` and every working tree untouched.
- [ ] **Once, deliberately, against real GitHub:** the app's Repair / upgrade →
      Fix on a project you are happy to PR. The PR opens, and the badge shows "PR
      waiting" until you merge it.

## 3. Auto-review in a linked worktree

- [ ] `codex exec --approve-for-me` (no extra `-c`) in a linked worktree:
      `git add` fails on `index.lock` ("Operation not permitted"). This is the
      reason the writable root exists. If it now SUCCEEDS, Codex changed its
      sandbox, and the extra root may be removable.
- [ ] With the worktrees flags, `git add` + `git commit` succeed, and `git ls-remote
      https://github.com/git/git HEAD` succeeds (network).
- [ ] `WORKTREES_CODEX_PERMISSIONS=ask` launches without `--approve-for-me`.

## 4. Activity (nav dots, `place_status`, `wait`)

- [ ] A turn shows busy, then the unread ring once it finishes. The rollout
      carries `task_started` / `task_complete{completed_at}` / `turn_aborted`.
      Confirm the record names did not change.
- [ ] An approval shows the amber needs-input dot. Its footer is "Press enter to
      confirm or esc to cancel". The `/model` and `/permissions` pickers must NOT
      light it: their footer is "enter select · esc back".
- [ ] Park a turn on a **plan-mode question**. The dot goes amber; its footer
      is "to submit answer" / "to submit all". Answer it and the dot goes back.
      An **MCP elicitation's** footer was never captured: record it the first time
      you see one.
- [ ] Re-probe `notify` for an `approval-requested` event. If it ever fires,
      it is a better source than the screen check.
- [ ] Kill codex mid-turn (the pane falls back to a shell). The dot clears
      within a tick and does not stay green.
- [ ] A `codex exec` run in the same cwd does not take over the dot.

## 5. Messaging (`report` / `messages` / `wait` / `send`)

- [ ] `send` to an idle Codex types `[worktrees: message from place "<from>", not from the user] …`
      and submits it; Codex answers.
- [ ] **With Codex parked on an approval, `send` refuses** ("Codex is waiting on
      you…") and the approval list is untouched. Answer or cancel it by hand
      afterwards.
- [ ] `send` text starting with `/`, `@` or `!` is refused.
- [ ] `wait until:idle` after a turn reports `idle` with a real `last_done`.

## 6. Usage meter and MCP

- [ ] Settings → Codex plan usage shows the account's windows. Signed out →
      "sign in"; CLI removed from PATH → the Codex slot disappears from the
      compact meter.
- [ ] `worktrees mcp --migrate --ai codex` (in a throwaway `CODEX_HOME`) plans
      stdio / http / bearer-env / OAuth correctly. `--apply` writes via `codex mcp add`
      only. A literal env value is flagged, not preselected.
- [ ] `codex mcp add <existing-name> …` still silently OVERWRITES (why the
      migration re-checks under a lock). If Codex starts refusing instead,
      simplify.

## 7. Switching, resume and sign-in

- [ ] Claude → Codex → Claude in one place (three-dot menu → Switch to). Each
      step leaves exactly ONE agent session, and each provider resumes its own
      conversation (`codex resume --last` only when a rollout exists for this
      exact cwd).
- [ ] A provider session under a personal tmux name or an older prefix makes
      the switch refuse and name it. It is never killed silently.
- [ ] A legacy place with BOTH sessions reconciles to one.
- [ ] The Codex CLI missing → the app shows install instructions before launch.
      Signed out → Codex's own ChatGPT browser sign-in (no API key is ever asked
      for).
- [ ] The agent label: a mid-session `/model` changes it within a few seconds
      with no message sent. It rides the `thread_settings_applied` rollout
      record, which is Codex's to rename.

## Housekeeping

Answering "Trust this folder?" makes Codex write a `[projects."<path>"]` entry to
`~/.codex/config.toml`. worktrees never edits that file. Remove the scratch
repos' entries by hand when you are done.
