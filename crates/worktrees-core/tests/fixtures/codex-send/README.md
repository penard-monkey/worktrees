Captured with `tmux capture-pane -p` from Codex CLI 0.157.1 on macOS,
2026-09-26, in a dedicated 110×35 scratch pane (`send-confirm-probe`).
The directory was `/private/tmp/worktrees-send-confirm-probe`.

- `empty.txt`: fresh composer, before typing.
- `typed.txt`: `tmux send-keys -l -- 'Reply only with OK.'`, without Enter.
- `pasted.txt`: after clearing the short input and typing a 1,282-character
  test message with `send-keys -l`, without Enter. The TUI's actual chip says
  `[Pasted Content 1284 chars]`; it is deliberately preserved verbatim.

Pressing Enter after the pasted capture produced an OK response and returned
an empty composer. Only trailing blank screen rows were removed; all visible text is preserved.
