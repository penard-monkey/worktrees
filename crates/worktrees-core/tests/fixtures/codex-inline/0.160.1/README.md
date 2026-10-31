Captured with `tmux capture-pane -p` from **codex-cli 0.160.1** on macOS
27.0, 2026-10-30, in a PRIVATE tmux server (120×40), launched as worktrees
launches it (`-c forced_login_method=chatgpt --no-alt-screen`) with
`-a on-request -s read-only` so that it asks, in scratch repositories under
`/private/tmp/claude-501/wtap/`. Only trailing blank rows were removed.

- `approval.txt`, `approval-no.txt`, `approval-declined.txt`: `touch b.txt`
  asks to run; the highlight on `3. No, and tell Codex what to do
  differently (esc)`; Enter on it interrupts the turn and returns to the
  composer. There is no field in the modal: "tell Codex" is the next line
  (`agent_send`).
- `approval-edit.txt`: an `apply_patch` (`Would you like to make the
  following edits?`). The diff is drawn as a HISTORY cell above the modal
  (`• Added notes.txt (+1 -0)`), so the modal alone does not show it. The
  rollout holds the pending call (`custom_tool_call` `exec` wrapping
  `tools.apply_patch(...)`) while it waits.
- `question-two.txt`, `question-two-second.txt`,
  `question-two-submitted.txt`: Plan mode's `request_user_input` with two
  questions — `Question 1/2 (2 unanswered)`, footer `tab to add notes |
  enter to submit answer | ←/→ to navigate questions | esc to interrupt`,
  then `Question 2/2` (`enter to submit all`), then the answers in history.
- `question-notes.txt`, `question-notes-typed.txt`: Tab on `None of the
  above` opens a notes row under it (`› Add notes`, footer `tab or esc to
  clear notes | enter to submit answer`); `teal please` pasted shows as
  `› teal please`, and was submitted with the answer (`note: teal please`).
