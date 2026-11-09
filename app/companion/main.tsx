// The Ideas companion window's entry (Ideas phase 0). Its own page and its own
// build (vite.companion.config.ts → app/dist/companion/), never a second input
// to the app's: the libraries ideas will need (CodeMirror, phase 1) live in
// this bundle and companion-boundary-check.mjs keeps them out of app/src's
// import graph. It imports app/src as SOURCE files — the door (ipc.ts), the
// socket client (ipcws.ts), the tokens — never the other way round.
//
// It is one more loopback client of this host's `worktrees serve`, through the
// same door as the main window: `engine_connect` for the port and token, then
// its own socket and handshake v1. One difference, and it is the retry
// policy's (reconnect.ts), not a copy of it: `neverStart`. The main window
// starts and restarts serve; this window only ever reconnects, so two windows
// never both restart a lost engine.
import React from "react";
import ReactDOM from "react-dom/client";
import type { SocketDeps } from "../src/ipcws";
import type { RetryEnv } from "../src/reconnect";

/** As main.tsx's: everything a socket to serve is built from but the port and
 *  the token, which come from `engine_connect` alone. */
function socketDeps(onError: (msg: string) => void): Omit<SocketDeps, "port" | "token"> {
  return {
    version: __WT_VERSION__,
    crypto: window.crypto,
    WebSocket: window.WebSocket,
    setTimeout: (fn, ms) => window.setTimeout(fn, ms),
    clearTimeout: (h) => window.clearTimeout(h as number),
    atob: (s) => window.atob(s),
    onError,
    warn: (m) => console.warn(`[ipc] ${m}`),
    origin: window.location.origin,
    remedy: "The main window restarts the engine; this window reconnects to it on its own.",
  };
}

/** As main.tsx's: a hidden window defers its retries until it is seen. */
function retryEnv(): RetryEnv {
  return {
    now: () => performance.now(),
    setTimeout: (fn, ms) => window.setTimeout(fn, ms),
    clearTimeout: (h) => window.clearTimeout(h as number),
    random: () => Math.random(),
    visible: () => document.visibilityState !== "hidden",
  };
}

async function boot() {
  if (import.meta.env.VITE_MOCK) {
    // The design harness: the same fake backend the main page installs, so
    // the page renders against it (it answers everything; no engine).
    await import("../src/mock/install");
  } else if ("__TAURI_INTERNALS__" in window) {
    const { engineDoor, openDoor, tauriDoor } = await import("../src/ipc");
    const { connectSocket } = await import("../src/ipcws");
    // `start` is always false here — the policy's `neverStart` — and the shim
    // obeys it: under false it starts and replaces nothing.
    const door = engineDoor(
      tauriDoor,
      async ({ reason, start, manual, onLost, onPhase }) => {
        const onPhaseCh = tauriDoor.channel<string>(onPhase);
        const ep = await tauriDoor.invoke<{ port: number; token: string; owned: boolean; started: boolean }>("engine_connect", { reason, start, manual, onPhase: onPhaseCh });
        const sock = connectSocket({ ...socketDeps(onLost), port: ep.port, token: ep.token });
        return { sock, owned: ep.owned, started: ep.started };
      },
      retryEnv(),
      { neverStart: true },
    );
    openDoor(door);
    document.addEventListener("visibilitychange", () => door.visibility());
    void door.connect();
  }
  const { Companion } = await import("./Companion");
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <Companion />
    </React.StrictMode>,
  );
  // The cross-window drag probe: dev builds only, and only when asked for.
  if (import.meta.env.DEV) {
    const { mountDragProbe } = await import("../src/dragprobe");
    mountDragProbe("companion");
  }
}

void boot();
