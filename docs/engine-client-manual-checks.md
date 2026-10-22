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

**Re-run this whenever `engine.rs`, `ipc.ts`'s `engineDoor`, `reconnect.ts`,
`EngineGate.tsx` or serve's `servefile.rs` / `--owner` change.**

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

### Automatic reconnection, verified the same way (2026-10-21)

Same setup, with `WORKTREES_SERVE_BIN` pointing at a stand-in that runs the
branch's release CLI unless a flag file exists (3c's script). No click was
made. There were 0 `[panic]` and 0 `[error]` lines in the bytes appended to
app.log and serve.log.

| Step | app.log says | Result |
|---|---|---|
| `kill -9` the app's serve | `automatic retry 1: the connection to worktrees serve closed…; may start: yes`, `pid not running → starting one`, `is ready` | Back in the same second, with no click. serve.log: the new serve's `client 2 connected`, exactly one page connection. |
| SIGTERM the app's serve | `serve pid=… exited (exit status: 0)`, `automatic retry 1`, `observed no serve.json → starting one` | serve stopped and removed serve.json before the 0.5 s retry, so there was nothing to wait for. |
| A third start inside one minute (launch 00:52:52, then 00:53:05 and 00:53:20; killed again at 00:53:44) | `automatic retry 1 …; may start: no`, `→ not starting one: this attempt may not` | The crash-loop limit stopped the retries. Nothing more was logged in the next 2 min 22 s. |
| Relaunch; `kill -9`, with serve.json swapped for one naming a live `sleep 3` on a closed port | `is alive with nothing on its port — waiting up to 10s…`, 3 s later `the old serve (pid …) stopped`, `pid not running → starting one`, `is ready` | The settle wait, live: no serve was started beside the "stopping" one. |
| A minute later: the stand-in's crash flag set, then `kill -9` | `started serve pid=…` then `exited before it was ready (exit status: 1)` at 00:58:10, :11 and :13; `automatic retry 4 …; may start: no`, `→ not starting one` at :15 | Stopped after three starts. 0 bytes were appended to app.log in the next 40 s. |

The banner itself was looked at in the dev stack (`server-stack.sh` +
`pnpm dev:serve`, the same door in headless Chromium), not in the app's
WKWebView. With the stack's serve killed it read "Reconnecting… Next try in
1 s." and then "…in 4 s.", in the warning tint, with the reason underneath
and **Reconnect now** enabled and focused. With the serve started again on a
new port, the page found it on its next try and the banner went. The only
console errors were Chromium's own "WebSocket connection … failed" lines, one
per try.

Not driven: anything that needs a click or the window's visibility (3b, 3d),
and the red "stopped" banner on screen. Those are the unit and check-script
tests (`reconnect-check.mjs`, `transport-check.mjs`), plus a hand on the steps
below.

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

### 3. Losing the engine: the app reconnects on its own

The retry policy is `app/src/reconnect.ts`: tries after 0.5, 1, 2, 4, 8, 16 s,
then every 30 s, each jittered down to between half and all of that. A try may
start serve only if the serve that went was the app's, and at most 3 starts in
any 60 s. **Space the sub-steps below more than a minute apart**: the launch's
own start counts, so three of them inside a minute is a crash loop by the
rules, and the app will stop instead of starting a fourth (that is 3c).

#### 3a. The engine dies; the app brings it back by itself

1. `kill -9 <pid from serve.json>`.
2. **Expect at once:** a banner at the top in the warning tint, "Reconnecting…
   Next try in 1 s", with **Reconnect now** focused. Within a second it reads
   "Starting the engine…" and then goes away. Do not press anything.
   - The place's terminal keeps its last screen. It does NOT look like a
     closed place.
   - The nav keeps its rows.
   - No error toasts pile up behind the banner.

**Expect in app.log, in order, with no click:**
- `engine: serve pid=<old> exited (signal: 9 (SIGKILL))`, but only if this app
  started that serve
- `engine: reconnect requested (automatic retry 1: the connection to worktrees
  serve closed. …); may start: yes`
- `observed serve(pid <old>…): pid not running`
- `→ starting one`, then `started serve pid=<new>`, then `is ready`

**Expect in the app:**
- The place's terminal redraws: tmux re-attaches.
- The dock shell comes back in its last directory, behind a
  `── restored · <time> ──` seam (the old serve's ring, flushed on its 15 s
  tick).
- Typing works again.

#### 3b. A serve someone else started is not restarted; Reconnect now

1. Quit the app (⌘Q). Start a serve by hand, as in step 5, and launch the app.
2. `kill <that serve's pid>`.

**Expect:**
- The banner reads "The engine was started outside this app, and it is not
  running. … Reconnect now starts the app's own." with a countdown that grows
  (1, 2, 4, 8, 16, then 30 s).
- app.log: `automatic retry …; may start: no`, then `→ not starting one: this
  attempt may not`, on every try.
- No `started serve` line.

3. Once the countdown shows 8 s or more, press **Reconnect now** (or Return).

**Expect:**
- app.log at once: `engine: reconnect requested on request (…); may start:
  yes`, then `started serve pid=<new> on request` and `is ready`.
- No `automatic retry` line after it: the pending timer was cancelled.
- The banner goes, and the app now owns that serve: a later `kill -9` of it is
  restarted on its own, as in 3a.

(Starting the hand-made serve again instead of pressing: the next try logs
`using serve(pid <pid>, …, owner none)` and the banner goes.)

#### 3c. A serve that keeps dying stops the retries

Use a stand-in serve that exits at once, so the real binary is never touched:

```sh
T=/tmp/claude-$(id -u)/ar; mkdir -p "$T"
cat > "$T/serve-bin" <<EOF
#!/bin/sh
[ -e "$T/crash" ] && { echo "stand-in serve: exiting at once" >&2; exit 1; }
exec "$PWD/target/release/worktrees" "\$@"
EOF
chmod 755 "$T/serve-bin"
WORKTREES_SERVE_BIN="$T/serve-bin" app/scripts/sandbox.sh --app   # sandbox.sh keeps a preset value
```

1. Wait more than a minute after launch, then `touch "$T/crash"` and
   `kill -9 <pid from serve.json>`.
2. Do not press anything.

**Expect:**
- app.log: three `started serve pid=… ` lines, each followed by `exited before
  it was ready`, at about 0.5, 1.5 and 3.5 s.
- Then one `may start: no` try, `→ not starting one`, and then NOTHING more,
  however long you wait.
- The banner turns red: "Disconnected from the engine. worktrees serve was
  started 3 times in the last 60 s and did not stay up, so the app has stopped
  starting it. Last: worktrees serve exited before it was ready … Its log:
  ~/.cache/worktrees/serve.log".
- `rm "$T/crash"`, then press **Reconnect now**: one `started serve … on
  request`, and the app is back.

#### 3d. A hidden window waits

1. Minimise the window (⌘M), then `kill -9 <pid from serve.json>` from a
   terminal.
2. Wait ten seconds, then bring the window back from the Dock.

**Expect:** no `reconnect requested` line in app.log while the window is
minimised, then one `automatic retry 1` line the moment it is visible again.

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
