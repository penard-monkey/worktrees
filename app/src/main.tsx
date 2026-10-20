import React from "react";
import ReactDOM from "react-dom/client";
import type { SocketDeps } from "./ipcws";

/** What every socket to `worktrees serve` is built from, whichever way the page
 *  learned the port and the token. */
function socketDeps(onError: (msg: string) => void, remedy?: string): Omit<SocketDeps, "port" | "token"> {
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
    remedy,
  };
}

/** The remedy the app offers for a server it will not drive: its own button. */
const RECONNECT = "Reconnect, and the app looks for worktrees serve again — and starts or replaces it if it has to.";

// Three ways to boot, chosen before anything that talks to the backend is
// imported: the design harness, the dev stack, and the app.
async function boot() {
  // Design harness (VITE_MOCK=1): install the fake Tauri backend BEFORE anything
  // imports @tauri-apps/api, so invoke()/Channel resolve against the mock +
  // fixtures. It answers everything itself; there is no engine to connect to.
  if (import.meta.env.VITE_MOCK) {
    await import("./mock/install");
  }
  let gated = false;
  // Dev stack (`pnpm dev:serve`, VITE_SERVE=1): a browser page on a real
  // `worktrees serve`, which learns the port and token from vite's token
  // endpoint (`bootSocket`). DEV is the literal `false` in a build, so this
  // block — and the endpoint's client with it — is dead code there;
  // transport-check.mjs greps app/dist to prove it. No local half: a browser
  // has no Tauri, so serve refuses the client-local commands, as it should.
  if (import.meta.env.DEV && import.meta.env.VITE_SERVE) {
    const { engineDoor, openDoor } = await import("./ipc");
    const { bootSocket } = await import("./ipcws");
    const door = engineDoor(null, (_reason, onLost) =>
      bootSocket({ ...socketDeps(onLost), fetch: (u, init) => window.fetch(u, init) }),
    );
    openDoor(door);
    void door.connect(null);
    gated = true;
  } else if (!import.meta.env.VITE_MOCK && "__TAURI_INTERNALS__" in window) {
    // The app: a client of this host's engine. The shim finds or starts
    // `worktrees serve` and answers `engine_connect` with its port and token —
    // the only way the token reaches this page, which keeps it in this closure
    // and the socket's HMAC key. Client-local commands stay on Tauri IPC.
    const { engineDoor, openDoor, tauriDoor } = await import("./ipc");
    const { connectSocket } = await import("./ipcws");
    const door = engineDoor(tauriDoor, async (reason, onLost) => {
      const ep = await tauriDoor.invoke<{ port: number; token: string }>("engine_connect", { reason });
      return connectSocket({ ...socketDeps(onLost, RECONNECT), port: ep.port, token: ep.token });
    });
    openDoor(door);
    void door.connect(null);
    gated = true;
  }
  const { startFeedback, stopFeedback } = await import("./feedback");
  const { FeedbackNotice } = await import("./FeedbackSection");
  void startFeedback();
  window.addEventListener("pagehide", stopFeedback, { once: true });
  if (import.meta.hot) import.meta.hot.dispose(stopFeedback);
  const { default: App } = await import("./App");
  // The gate holds the app back until the engine has answered once — App
  // hydrates its settings on mount, and against a dead engine it would hydrate
  // DEFAULTS and later save them over the real ones — and after that keeps it
  // mounted under a "disconnected" banner rather than blanking it.
  const { EngineGate } = await import("./EngineGate");
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      {gated ? (
        <EngineGate>
          <App />
        </EngineGate>
      ) : (
        <App />
      )}
      <FeedbackNotice />
    </React.StrictMode>,
  );
}

void boot();
