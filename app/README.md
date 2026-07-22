# worktrees UI — P1 spike

A Tauri (Rust + React/TS) desktop app on top of the `worktrees` CLI. This phase
(**P1**) proves the load-bearing risk from [`../DESIGN.md`](../DESIGN.md): an
embedded tmux terminal that **attaches** to a live session (never owns a shell),
so closing the app **detaches** and the session survives.

## What's here

- **Rust** (`src-tauri/src/lib.rs`) — the engine bridge, no git/tmux logic of its own:
  - `list_places(repo)` — shells out to `worktrees ls --json`, returns it verbatim.
  - `term_open/term_write/term_resize/term_close` — a `portable-pty` child running
    `tmux attach-session -t <session>`, streamed to the UI over a Tauri Channel
    (raw bytes). Close kills the *client* process = detach, never the session.
- **Frontend** (`src/`) — left nav of places from `ls --json`, top bar (branch /
  ahead-behind / lifecycle), and an `xterm.js` pane wired to the PTY commands.

## Run (dev)

Requires: Rust, Node, pnpm, tmux, git on PATH (macOS/Linux — Windows is a non-goal).

The app links `worktrees-core` in-process (no subprocess, no `WORKTREES_BIN`); core
shells out to `git`/`tmux` directly, so just:

```sh
pnpm --dir app tauri dev
```

## Verify P1 (exit criteria)

1. Create a place with a live tmux session (plain shell, no AI, for the demo):

   ```sh
   ./bin/worktrees new spike-demo --no-attach --ai none
   ```

2. In the app, keep the repo path (defaults to this repo) and hit ↻. The place
   appears with a green dot (live tmux).
3. Click it — the tmux session embeds on the right. Type; run commands.
4. Resize the window — the terminal reflows without garble.
5. Quit the app, then from a bare terminal:

   ```sh
   tmux attach -t worktrees-spike-demo
   ```

   The **same session is still alive** — close was a detach, not a kill.
6. Reopen the app, click the place → it reattaches.

## Not yet (later phases)

Declared-state store + lifecycle writes (P2), infra up/down (P3), grouped-session
resize for multi-client, polish/packaging (P4). See [`../DESIGN.md`](../DESIGN.md).
