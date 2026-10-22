---
title: "The app as serve's client — manual verification checklist"
---

# The app as serve's client — manual verification checklist

The desktop app no longer runs the engine itself. It finds or starts this
host's `worktrees serve` (`app/src-tauri/src/engine.rs`) and talks to it over
an authenticated loopback WebSocket. Everything except the window and the six
client-local commands goes through serve.

The unit tests prove the decision table (`cargo test -p app --lib engine::`).
The check scripts prove the routing and the wiring (`engine-route-check.mjs`,
`api-boundary-check.mjs`, `transport-check.mjs`). None of them presses a
button in the real app, and AGENTS.md forbids scripting clicks or keys into a
sandbox app. So the steps below that need a hand are run by a person. The lane
reads the logs afterwards.

**Re-run this whenever `engine.rs`, `ipc.ts`'s `engineDoor`, `EngineGate.tsx`
or serve's `servefile.rs` / `--owner` change.**

## What the lane already verified without a click (2026-10-20)

Run on macOS 27.0 in `sandbox.sh --app` (tauri dev, so the page origin was
`http://localhost:1420`), on an isolated tmux server, with
`WORKTREES_SERVE_BIN` pointing at this branch's release CLI. There were
0 `[panic]` lines in app.log and serve.log.

| Step | app.log says | Result |
|---|---|---|
| Launch, no serve.json | `no serve.json → starting one`, `started serve pid=4718 … dev_origin=http://localhost:1420`, `is ready on 127.0.0.1:63484 — this app stops it on quit` | serve.json 0600 with `owner: "app"`. The page connected (serve.log `client 2 connected`). serve's children were the place's tmux attach and the dock shell. |
| App killed (SIGTERM, which is a crash: no Exit arm runs) | — | serve survived. serve.log `client 2 disconnected (1 attach(es) closed)`. The dock shell stayed alive. |
| Relaunch | `proven v0.42.0`, `adopting serve(pid 4718 …, owner app) — … this app stops it on quit` | The tmux attach was re-made and the same dock shell was re-attached. |
| A v0.42.0-mm serve with no owner on the config dir, then launch | `proved v0.42.0-mm, this app is v0.42.0 → replacing it`, `the old serve (pid 17104) stopped`, `started serve pid=19415`, `is ready` | serve.log for 17104: `stopping`, `stopped`. It was SIGTERMed, never SIGKILLed. |
| A v0.42.0 serve with no owner (started the CLI way), then launch | `using serve(pid 21881 …, owner none), started by someone else — it stays running when the app quits` | It was used. A fresh dock shell replayed the saved ring. |

## Setup

```sh
app/scripts/sandbox.sh --app        # builds the CLI, launches tauri dev under the .sbx identifier
```

Keep two terminals open on the logs, reading only what is appended from now on:

```sh
tail -n0 -F ~/Library/Logs/net.casadelvalle.worktrees.sbx/app.log
tail -n0 -F ~/.cache/worktrees/serve.log        # every line carries the writing serve's pid
```

`serve.json` lives in `~/Library/Application Support/net.casadelvalle.worktrees.sbx/`.
Its `pid` is the serve to watch. The token is in that file too; do not paste
the file anywhere.

## Checks that need a hand

### 1. The app lists the workspace and its terminals work

1. Add the sandbox repo, if it is not listed: Add project, then
   `~/.cache/worktrees/sandbox/<slug>/repo`.
2. Make a place, select it, and Enter ▸ it. Its terminal shows the session.
3. Open the dock's Terminal tab and type `echo hello`, then press Return.

**Expect:** `hello` echoes in the dock. Nothing new appears in app.log for any
of this. Those commands went to serve, not the shim.

### 2. One mutation

Give the place a note (the place header's note field) and press Return.

**Expect:** the note shows in the nav. `<repo>/.worktrees.places.json` holds it.

### 3. Losing the engine, and Reconnect

1. `kill -9 <pid from serve.json>`.
2. **Expect at once:** a red banner at the top, "Disconnected from the engine.
   the connection to worktrees serve closed. Reconnect, and the app looks for
   worktrees serve again…", with **Reconnect** focused.
   - The place's terminal keeps its last screen. It does NOT look like a
     closed place.
   - The nav keeps its rows.
   - No error toasts pile up behind the banner.
   - app.log: `engine: serve pid=<pid> exited (signal: 9 …)`, but only if this
     app started that serve.
3. Press **Reconnect** (or Return).

**Expect:**
- app.log, in order:
  - `engine: reconnect requested (the connection to worktrees serve closed…)`
  - `observed serve(pid <old>…): pid not running`
  - `→ starting one`
  - `started serve pid=<new>`
  - `is ready`
- The banner goes.
- The place's terminal redraws: tmux re-attaches.
- The dock shell comes back in its last directory, behind a
  `── restored · <time> ──` seam (the old serve's ring, flushed on its 15 s
  tick).
- Typing works again.

### 4. A clean quit stops the serve the app manages

⌘Q.

**Expect:**
- app.log: `engine: quitting; serve pid=<pid> stopped`. If it says "still
  flushing", it finishes on its own.
- serve.log for that pid: `serve: stopping`, then `serve: stopped`.
- serve.json is gone.
- `pgrep -fl "worktrees serve"` lists none for the .sbx config dir.

### 5. A serve someone else started is used and left running

From the repo root, with the app quit:

```sh
R=$PWD CFG="$HOME/Library/Application Support/net.casadelvalle.worktrees.sbx"
( eval "$(app/scripts/sandbox.sh)"   # the sandbox's XDG dirs and prefix, so serve reads its registry
  nohup python3 -c 'import os,sys; os.setsid(); os.execvp(sys.argv[1], sys.argv[1:])' \
    "$R/target/release/worktrees" serve --config-dir "$CFG" --allow-origin http://localhost:1420 \
    >/dev/null 2>&1 </dev/null & )
```

1. Launch with `app/scripts/sandbox.sh --app`.
   **Expect:** app.log `using serve(pid <pid>, …, owner none), started by
   someone else — it stays running when the app quits`.
2. Then ⌘Q.
   **Expect:** app.log `engine: quitting; the serve in use is not this app's
   to stop`, and that serve is still running. Stop it with `kill <pid>`.

### 6. A crash leftover is adopted, then stopped on quit

1. Launch with `sandbox.sh --app`. The app starts serve and owns it.
2. `kill -9 <the app's pid>`. Find it with `pgrep -fl 'target/debug/app'` and
   confirm with `ps -o pid,command`. Never address the app by name.
3. **Expect:** serve is still running, and its dock shell is still alive.
4. Relaunch within two minutes.
   **Expect:** app.log `adopting serve(pid <pid>, …, owner app)`.
5. ⌘Q.
   **Expect:** app.log `engine: quitting; serve pid=<pid> stopped`.
6. Again from step 1, but leave it killed for more than two minutes.
   **Expect:** serve.log `serve: no client for 120s, and the app that owns
   this serve is gone — stopping`, then `serve: stopping` and `serve:
   stopped`; serve.json is gone. A relaunch starts a fresh serve, and the
   dock tab comes back from its saved scrollback behind a `── restored ──`
   seam.

## What is not covered here

- **The shipped app on `tauri://localhost`.** This branch's bundle has no CLI
  in it: the sidecar is #513. Every run above is `tauri dev` on
  `http://localhost:1420`. A release build starts serve with no
  `--allow-origin` and probes with `tauri://localhost`, which serve allows by
  name. That path has been checked in the source (`engine-route-check.mjs`)
  but has not been run.
- **Linux.** The decision table, the observation and the binary layouts are
  unit tests that CI runs on ubuntu. Nothing above was run on Linux.
