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

## Send/wait verification on 2026-09-27

The `probe-*` captures come from dedicated 120×40 scratch tmux sessions on
macOS, running codex-cli 0.157.1 with `--no-daemon --no-alt-screen`,
workspace-write sandboxing and on-request/user approvals. The scratch git
repository had separate busy, approval and question worktrees. No user lanes
were driven. Only trailing blank rows were removed from these captures.

- `probe-busy.txt`: a turn running `sleep 300`.
- `probe-approval.txt`: a pending escalation for harmless `pwd`.
- `probe-question-modal.txt`: Plan mode's blocking `request_user_input`.
- `probe-question-banner-empty.txt`: `request_user_input_async` leaves a
  pending `? 1 question` / `shift+← to answer` banner above an empty composer
  while the agent continues working. This is not a blocking modal.
- `probe-question-banner-typed.txt`: a short message typed with `send-keys -l`,
  without Enter, under that banner. The footer becomes the queue hint.
- `probe-question-banner-pasted.txt`: a long message typed without Enter under
  a second async question. The actual chip reads `[Pasted Content 1163 chars]`.
- `probe-question-submitted.txt`: immediately after the release MCP's `send`
  submitted a long attributed message with the banner present. The message is
  in history and a new empty composer is visible; Codex later replied RECEIVED.
- `probe-task-started.jsonl`: a real turn-start record from the busy scratch
  session, reserialized as compact JSON. No prompt or credentials are included.

The baseline was commit `3514559` (v0.32.1), freshly built with
`cargo build --release -p worktrees-cli`. Each probe called that binary's
`mcp --mutations` over stdio from the scratch main checkout. Zero-timeout
`wait {until:"idle"}` returned `event:timeout, activity.state:busy` for the
running sleep and async-question turn, and `event:waiting,
activity.state:waiting` for both blocking modals. Sending into the approval
was refused. Both a short mid-turn send and a long send under the question
banner returned `delivered:true` with the confirmed-submission note.

Verdicts: neither reported failure reproduced on current main. The reported
false idle is already covered by the shared activity reader introduced in
#348 and retained through #351; #352 fixes the false delivery report. The absent banner-string match is
intentional here: submission is determined by the live composer below it.
The regressions also exercise a lost Enter and a composer that never clears;
the latter must exhaust the bounded retries with `Unconfirmed`.
