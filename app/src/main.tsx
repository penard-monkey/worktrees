import React from "react";
import ReactDOM from "react-dom/client";

// Design harness (VITE_MOCK=1): install the fake Tauri backend BEFORE anything
// imports @tauri-apps/api, so invoke()/Channel resolve against the mock + fixtures.
async function boot() {
  if (import.meta.env.VITE_MOCK) {
    await import("./mock/install");
  }
  const { default: App } = await import("./App");
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

void boot();
