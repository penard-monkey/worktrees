import React from "react";
import ReactDOM from "react-dom/client";

// Design harness (VITE_MOCK=1): install the fake Tauri backend BEFORE anything
// imports @tauri-apps/api, so invoke()/Channel resolve against the mock + fixtures.
async function boot() {
  if (import.meta.env.VITE_MOCK) {
    await import("./mock/install");
  }
  // Dev stack (`pnpm dev:serve`, VITE_SERVE=1): the backend is a real
  // `worktrees serve` over a WebSocket rather than Tauri IPC (ipc.ts). DEV is
  // the literal `false` in a build, so this block — and the socket client with
  // it — is dead code there; transport-check.mjs greps app/dist to prove it.
  if (import.meta.env.DEV && import.meta.env.VITE_SERVE) {
    const { openDoor, transportError } = await import("./ipc");
    const { bootSocket } = await import("./ipcws");
    openDoor(await bootSocket({
      version: __WT_VERSION__,
      fetch: (u, init) => window.fetch(u, init),
      WebSocket: window.WebSocket,
      setTimeout: (fn, ms) => window.setTimeout(fn, ms),
      clearTimeout: (h) => window.clearTimeout(h as number),
      atob: (s) => window.atob(s),
      onError: transportError,
      warn: (m) => console.warn(`[ipc] ${m}`),
      origin: window.location.origin,
    }));
  }
  const { startFeedback, stopFeedback } = await import("./feedback");
  const { FeedbackNotice } = await import("./FeedbackSection");
  void startFeedback();
  window.addEventListener("pagehide", stopFeedback, { once: true });
  if (import.meta.hot) import.meta.hot.dispose(stopFeedback);
  const { default: App } = await import("./App");
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
      <FeedbackNotice />
    </React.StrictMode>,
  );
}

void boot();
