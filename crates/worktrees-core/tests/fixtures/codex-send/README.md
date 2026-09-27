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

The six `review-*.txt` captures were supplied in the #352 review on 2026-09-26
from throwaway Codex 0.157.1 panes on this machine. They retain the review's
shortened paths, with only trailing blank rows removed. `review-busy-typed`
shows the queue hint; `review-busy-queued`, both idle screens, and
`review-post-final` show the Agent Command Center navigation hint.
`review-typed400` covers wrapped, nonempty input.

`busy-no-status-{typed,queued}.txt` were captured during the review fixes on
2026-09-26 from the same dedicated scratch pane, running Codex 0.157.1 with
the shared daemon. While a `sleep 30` turn ran, typing replaced the whole
model/path status with the queue hint and context percentage; after Enter,
the queue banner appeared and the empty composer regained its status line.
