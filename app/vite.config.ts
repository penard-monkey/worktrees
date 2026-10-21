import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { serveDoor } from "./vite.serve-door";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
// `pnpm dev:serve`: the frontend drives a real `worktrees serve` (ipc.ts).
// @ts-expect-error process is a nodejs global
const serve = !!process.env.VITE_SERVE;

/** This frontend's own version, for the serve handshake (`__WT_VERSION__`). The
 *  workspace Cargo.toml is the repo's one version source — the app crate and
 *  tauri.conf.json already inherit it, and the server's auth reply carries the
 *  CLI's `CARGO_PKG_VERSION`, which inherits it too. Read at config time, so a
 *  bundle carries the version it was BUILT from, which is the one to compare. */
function workspaceVersion(): string {
  const toml = fs.readFileSync(fileURLToPath(new URL("../Cargo.toml", import.meta.url)), "utf8");
  const section = toml.split(/^\[workspace\.package\]\s*$/m)[1]?.split(/^\[/m)[0] ?? "";
  const v = /^version\s*=\s*"([^"]+)"/m.exec(section)?.[1];
  if (!v) throw new Error("vite.config.ts: no [workspace.package] version in ../Cargo.toml");
  return v;
}

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), ...(serve ? [serveDoor()] : [])],
  define: { __WT_VERSION__: JSON.stringify(workspaceVersion()) },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
