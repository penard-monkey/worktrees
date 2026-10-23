import React from "react";
import ReactDOM from "react-dom/client";
import type { Door, EngineDoor } from "./ipc";
import type { HostSummary, OpenPipe } from "./ipcremote";
import type { SocketDeps } from "./ipcws";
import type { RetryEnv } from "./reconnect";

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

/** The remedy the app offers for a server it will not drive: its own button.
 *  Only what is true in every state the banner can be in — retrying, waiting
 *  for the window, or stopped — so nothing about trying on its own. */
const RECONNECT = "Reconnect now tries again at once.";

/** The clock, chance and visibility the engine door's retries run on
 *  (reconnect.ts). `performance.now()`, not `Date.now()`: a wall-clock jump
 *  must not fire or starve a retry, and EngineGate's countdown reads the same
 *  clock. */
function retryEnv(): RetryEnv {
  return {
    now: () => performance.now(),
    setTimeout: (fn, ms) => window.setTimeout(fn, ms),
    clearTimeout: (h) => window.clearTimeout(h as number),
    random: () => Math.random(),
    visible: () => document.visibilityState !== "hidden",
  };
}

/** The paired host the window was last told to drive (Settings → Hosts), or
 *  null for this machine's own engine — also when the host list cannot be
 *  read, which Settings shows. */
async function chosenHost(door: Door): Promise<HostSummary | null> {
  try {
    const h = await door.invoke<{ chosen: string | null; hosts: HostSummary[] }>("remote_hosts");
    return (h.chosen && h.hosts.find((x) => x.id === h.chosen)) || null;
  } catch (e) {
    console.warn(`[hosts] ${String(e)} — this window drives this machine's own engine`);
    return null;
  }
}

/** The door of a window that drives a paired host: ipcws.ts, unchanged, over
 *  the shim's pinned pipe (ipcremote.ts), keyed with the pipe's session token.
 *  Never `engine_connect`: with a host chosen, no local serve is found or
 *  started (§4.4). `owned: false`, so the retries reconnect and never start. */
async function remoteBoot(host: HostSummary): Promise<EngineDoor> {
  const { remoteDoor, setRemoteGrant, tauriDoor } = await import("./ipc");
  // Destructured, never a namespace binding: a dynamic import Rollup sees used
  // as a whole object keeps EVERY export of ipcws, the dev stack's token client
  // (`bootSocket`) included, in the production chunk (transport-check).
  const { connectSocket: connectRemote } = await import("./ipcws");
  const { openPipe, shimOf } = await import("./ipcremote");
  const shim = shimOf(tauriDoor);
  const remedy =
    `Reconnect now tries again at once; each try looks for ${host.name} on the LAN if it is not where it was. ` +
    `If it moved, paste its address code in Settings → Hosts → Paste a new address; if it no longer knows this machine, pair again there.`;
  return remoteDoor(
    tauriDoor,
    host.name,
    async ({ onLost }) => {
      let pipe: OpenPipe | null = null;
      // The shim's words for an end (a revoke says so) go with ipcws's.
      const lost = (msg: string) => {
        const why = pipe?.closedWhy();
        onLost(why && !msg.includes(why) ? `${msg} (${why})` : msg);
      };
      // A fresh grant per socket: a narrowed device learns it on reconnect.
      setRemoteGrant(null);
      pipe = await openPipe(shim, host.id, { setTimeout: (fn, ms) => window.setTimeout(fn, ms) }, setRemoteGrant);
      const sock = connectRemote({ ...socketDeps(lost, remedy), port: 0, token: pipe.connected.token, remoteHost: pipe.remoteHost, WebSocket: pipe.WebSocket });
      return { sock, owned: false, started: false };
    },
    retryEnv(),
  );
}

// Three ways to boot, chosen before anything that talks to the backend is
// imported: the design harness, the dev stack, and the app.
async function boot() {
  // Design harness (VITE_MOCK=1): install the fake Tauri backend BEFORE anything
  // imports @tauri-apps/api, so invoke()/Channel resolve against the mock +
  // fixtures. It answers everything itself; there is no engine to connect to,
  // unless `?slowengine` puts an engine door in front of it.
  let gated = false;
  if (import.meta.env.VITE_MOCK) {
    const { mockDeclare, mockEngineDelayMs } = await import("./mock/install");
    // `?remote`: as if a paired host were chosen, so every act that needs the
    // host's paths shows greyed, with its reason (§4.4).
    // `?remote=noadmin`: …and a device grant without admin, so the sections
    // that need it say so.
    const remote = new URLSearchParams(window.location.search).get("remote");
    if (remote !== null) {
      const { markRemote, setRemoteGrant } = await import("./ipc");
      markRemote("Studio");
      if (remote === "noadmin") setRemoteGrant(["read", "write", "stream"]);
    }
    // `?slowengine[=ms]`: the app's boot over the mock — an engine door whose
    // first connect takes that long, so a call that cannot wait for it is
    // refused here exactly as it is in the app. Its socket is the mock itself.
    if (mockEngineDelayMs > 0) {
      const { engineDoor, openDoor, tauriDoor } = await import("./ipc");
      const door = engineDoor(
        tauriDoor,
        async () => {
          await new Promise((r) => window.setTimeout(r, mockEngineDelayMs));
          // `declare` tells the mock what the door listens for, as it tells
          // serve: a held show waits for it, not for the door's own wiring.
          return { sock: { ...tauriDoor, ready: Promise.resolve(), declare: mockDeclare }, owned: true, started: false };
        },
        retryEnv(),
      );
      openDoor(door);
      void door.connect();
      gated = true;
    }
  }
  // Dev stack (`pnpm dev:serve`, VITE_SERVE=1): a browser page on a real
  // `worktrees serve`, which learns the port and token from vite's token
  // endpoint (`bootSocket`). DEV is the literal `false` in a build, so this
  // block — and the endpoint's client with it — is dead code there;
  // transport-check.mjs greps app/dist to prove it. No local half: a browser
  // has no Tauri, so serve refuses the client-local commands, as it should.
  if (import.meta.env.DEV && import.meta.env.VITE_SERVE) {
    const { engineDoor, openDoor, setAway } = await import("./ipc");
    // A browser has no opener or dialog plugin to act on a path with (§4.4).
    setAway("this page runs in a browser, which cannot open the host's files");
    const { bootSocket } = await import("./ipcws");
    // The stack's serve is server-stack.sh's, never the page's: owned false,
    // so a lost one is only ever reconnected to (re-reading the endpoint).
    const door = engineDoor(
      null,
      async ({ onLost }) => ({
        sock: await bootSocket({ ...socketDeps(onLost), fetch: (u, init) => window.fetch(u, init) }),
        owned: false,
        started: false,
      }),
      retryEnv(),
    );
    openDoor(door);
    document.addEventListener("visibilitychange", () => door.visibility());
    void door.connect();
    gated = true;
  } else if (!import.meta.env.VITE_MOCK && "__TAURI_INTERNALS__" in window) {
    // The app: a client of this host's engine. The shim finds or starts
    // `worktrees serve` and answers `engine_connect` with its port and token —
    // the only way the token reaches this page, which keeps it in this closure
    // and the socket's HMAC key. Client-local commands stay on Tauri IPC.
    const { engineDoor, openDoor, tauriDoor } = await import("./ipc");
    const { connectSocket } = await import("./ipcws");
    // Which engine: this machine's own, or a paired host (Settings → Hosts).
    const host = await chosenHost(tauriDoor);
    // `start` is the retry policy's (reconnect.ts) and the shim obeys it: under
    // false it starts and replaces nothing. `owned`/`started` are the shim's
    // answer, and the policy's only source for either.
    const door = host
      ? await remoteBoot(host)
      : engineDoor(
          tauriDoor,
          async ({ reason, start, manual, onLost, onPhase }) => {
            const onPhaseCh = tauriDoor.channel<string>(onPhase);
            const ep = await tauriDoor.invoke<{ port: number; token: string; owned: boolean; started: boolean }>("engine_connect", { reason, start, manual, onPhase: onPhaseCh });
            const sock = connectSocket({ ...socketDeps(onLost, RECONNECT), port: ep.port, token: ep.token });
            return { sock, owned: ep.owned, started: ep.started };
          },
          retryEnv(),
        );
    openDoor(door);
    document.addEventListener("visibilitychange", () => door.visibility());
    void door.connect();
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
