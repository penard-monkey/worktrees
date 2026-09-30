Captured with `tmux capture-pane -p` from pi 0.99.1 on macOS, 2026-09-29, in a
dedicated 120×40 scratch pane on a throwaway tmux server (`-L pisend`), with a
throwaway `PI_CODING_AGENT_DIR` and `lm-studio/qwen/qwen3-coder-480b`. Only
trailing blank rows were removed, and the cwd footer was shortened to
`~/scratch/repo/.worktrees/lane`.

Each is what `send` sees, with the attributed header it types:

- `idle-empty.txt`: pi at rest, empty composer.
- `idle-typed.txt`: `send-keys -l` of a message, no Enter yet; it sits inline
  between the composer's two rules.
- `idle-submitted.txt`: 0.4s after Enter; the composer is empty and its top
  border carries `Working`. (The session file got the user entry at submit.)
- `busy-typed.txt`: a second message typed while that turn runs.
- `busy-queued.txt`: 0.5s after its Enter: the composer is empty and the
  message is queued above the border as `Steering: …`, with pi's
  `↳ Option+Up to edit all queued messages` hint. The session file does NOT
  have it yet — pi writes a steering message when it DELIVERS it.
- `busy-typed-long.txt`: a ~1,100-character one-line message typed while
  busy; pi wraps it inline and does not fold it.
